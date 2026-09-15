// Standard raw binary buffer manipulation library for Neyuki VM.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::buffer::VmBuffer;
use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

fn get_buf<'a>(val: &'a Value) -> Result<&'a Rc<RefCell<VmBuffer>>, String> {
    match val {
        Value::Buffer(b) => Ok(b),
        _ => Err("buffer operation expects a buffer instance".to_string()),
    }
}

fn to_usize(val: &Value, name: &str) -> Result<usize, String> {
    match val {
        Value::Int(i) => i.to_usize().ok_or_else(|| format!("{} is out of bounds", name)),
        Value::Float(f) => {
            if *f < 0.0 {
                Err(format!("{} cannot be negative", name))
            } else {
                Ok(*f as usize)
            }
        }
        _ => Err(format!("{} expects an integer", name)),
    }
}

fn buf_create(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let size = to_usize(args.first().ok_or_else(|| "buffer.create expects size".to_string())?, "size")?;
    let buf = VmBuffer::new(size);
    Ok(vec![Value::Buffer(Rc::new(RefCell::new(buf)))])
}

fn buf_fromstring(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = match args.first().ok_or_else(|| "buffer.fromstring expects string".to_string())? {
        Value::String(s) => s.as_bytes().to_vec(),
        _ => return Err("buffer.fromstring expects string".to_string()),
    };
    let buf = VmBuffer::from_bytes(s);
    Ok(vec![Value::Buffer(Rc::new(RefCell::new(buf)))])
}

fn buf_tostring(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.tostring expects buffer".to_string())?)?;
    let s = String::from_utf8_lossy(&buf_rc.borrow().data).to_string();
    Ok(vec![Value::String(s)])
}

fn buf_len(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.len expects buffer".to_string())?)?;
    let len = buf_rc.borrow().len();
    Ok(vec![Value::Int(BigInt::from(len))])
}

fn buf_readu8(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.readu8 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.readu8 expects offset".to_string())?, "offset")?;
    let val = buf_rc.borrow().read_u8(offset)?;
    Ok(vec![Value::Int(BigInt::from(val))])
}

fn buf_writeu8(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.writeu8 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.writeu8 expects offset".to_string())?, "offset")?;
    let val = to_usize(args.get(2).ok_or_else(|| "buffer.writeu8 expects value".to_string())?, "value")? as u8;
    buf_rc.borrow_mut().write_u8(offset, val)?;
    Ok(vec![])
}

fn buf_readi8(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.readi8 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.readi8 expects offset".to_string())?, "offset")?;
    let val = buf_rc.borrow().read_i8(offset)?;
    Ok(vec![Value::Int(BigInt::from(val))])
}

fn buf_writei8(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.writei8 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.writei8 expects offset".to_string())?, "offset")?;
    let val = to_usize(args.get(2).ok_or_else(|| "buffer.writei8 expects value".to_string())?, "value")? as i8;
    buf_rc.borrow_mut().write_i8(offset, val)?;
    Ok(vec![])
}

fn buf_readu16(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.readu16 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.readu16 expects offset".to_string())?, "offset")?;
    let val = buf_rc.borrow().read_u16(offset)?;
    Ok(vec![Value::Int(BigInt::from(val))])
}

fn buf_writeu16(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.writeu16 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.writeu16 expects offset".to_string())?, "offset")?;
    let val = to_usize(args.get(2).ok_or_else(|| "buffer.writeu16 expects value".to_string())?, "value")? as u16;
    buf_rc.borrow_mut().write_u16(offset, val)?;
    Ok(vec![])
}

fn buf_readi16(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.readi16 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.readi16 expects offset".to_string())?, "offset")?;
    let val = buf_rc.borrow().read_i16(offset)?;
    Ok(vec![Value::Int(BigInt::from(val))])
}

fn buf_writei16(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.writei16 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.writei16 expects offset".to_string())?, "offset")?;
    let val = to_usize(args.get(2).ok_or_else(|| "buffer.writei16 expects value".to_string())?, "value")? as i16;
    buf_rc.borrow_mut().write_i16(offset, val)?;
    Ok(vec![])
}

fn buf_readu32(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.readu32 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.readu32 expects offset".to_string())?, "offset")?;
    let val = buf_rc.borrow().read_u32(offset)?;
    Ok(vec![Value::Int(BigInt::from(val))])
}

fn buf_writeu32(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.writeu32 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.writeu32 expects offset".to_string())?, "offset")?;
    let val = to_usize(args.get(2).ok_or_else(|| "buffer.writeu32 expects value".to_string())?, "value")? as u32;
    buf_rc.borrow_mut().write_u32(offset, val)?;
    Ok(vec![])
}

fn buf_readi32(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.readi32 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.readi32 expects offset".to_string())?, "offset")?;
    let val = buf_rc.borrow().read_i32(offset)?;
    Ok(vec![Value::Int(BigInt::from(val))])
}

fn buf_writei32(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.writei32 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.writei32 expects offset".to_string())?, "offset")?;
    let val = to_usize(args.get(2).ok_or_else(|| "buffer.writei32 expects value".to_string())?, "value")? as i32;
    buf_rc.borrow_mut().write_i32(offset, val)?;
    Ok(vec![])
}

fn buf_readf32(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.readf32 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.readf32 expects offset".to_string())?, "offset")?;
    let val = buf_rc.borrow().read_f32(offset)?;
    Ok(vec![Value::Float(val as f64)])
}

fn buf_writef32(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.writef32 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.writef32 expects offset".to_string())?, "offset")?;
    let f = match args.get(2).ok_or_else(|| "buffer.writef32 expects value".to_string())? {
        Value::Float(f) => *f as f32,
        Value::Int(i) => i.to_f32().unwrap_or(0.0),
        _ => return Err("buffer.writef32 expects float value".to_string()),
    };
    buf_rc.borrow_mut().write_f32(offset, f)?;
    Ok(vec![])
}

fn buf_readf64(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.readf64 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.readf64 expects offset".to_string())?, "offset")?;
    let val = buf_rc.borrow().read_f64(offset)?;
    Ok(vec![Value::Float(val)])
}

fn buf_writef64(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.writef64 expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.writef64 expects offset".to_string())?, "offset")?;
    let f = match args.get(2).ok_or_else(|| "buffer.writef64 expects value".to_string())? {
        Value::Float(f) => *f,
        Value::Int(i) => i.to_f64().unwrap_or(0.0),
        _ => return Err("buffer.writef64 expects float value".to_string()),
    };
    buf_rc.borrow_mut().write_f64(offset, f)?;
    Ok(vec![])
}

fn buf_readstring(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.readstring expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.readstring expects offset".to_string())?, "offset")?;
    let count = to_usize(args.get(2).ok_or_else(|| "buffer.readstring expects count".to_string())?, "count")?;
    let s = buf_rc.borrow().read_string(offset, count)?;
    Ok(vec![Value::String(s)])
}

fn buf_writestring(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.writestring expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.writestring expects offset".to_string())?, "offset")?;
    let s = match args.get(2).ok_or_else(|| "buffer.writestring expects string".to_string())? {
        Value::String(s) => s,
        _ => return Err("buffer.writestring expects string".to_string()),
    };
    buf_rc.borrow_mut().write_string(offset, s)?;
    Ok(vec![])
}

fn buf_copy(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let target = get_buf(args.first().ok_or_else(|| "buffer.copy expects target".to_string())?)?;
    let target_offset = to_usize(args.get(1).ok_or_else(|| "buffer.copy expects target offset".to_string())?, "target_offset")?;
    let source = get_buf(args.get(2).ok_or_else(|| "buffer.copy expects source".to_string())?)?;
    let source_offset = to_usize(args.get(3).ok_or_else(|| "buffer.copy expects source offset".to_string())?, "source_offset")?;
    let count = to_usize(args.get(4).ok_or_else(|| "buffer.copy expects count".to_string())?, "count")?;
    target.borrow_mut().copy(target_offset, &source.borrow(), source_offset, count)?;
    Ok(vec![])
}

fn buf_fill(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let buf_rc = get_buf(args.first().ok_or_else(|| "buffer.fill expects buffer".to_string())?)?;
    let offset = to_usize(args.get(1).ok_or_else(|| "buffer.fill expects offset".to_string())?, "offset")?;
    let val = to_usize(args.get(2).ok_or_else(|| "buffer.fill expects value".to_string())?, "value")? as u8;
    let count = to_usize(args.get(3).ok_or_else(|| "buffer.fill expects count".to_string())?, "count")?;
    buf_rc.borrow_mut().fill(offset, val, count)?;
    Ok(vec![])
}

pub fn create_buffer_lib() -> Value {
    let mut table = VmTable::new();
    table.set_str("create", Value::Native("buffer.create", buf_create));
    table.set_str("fromstring", Value::Native("buffer.fromstring", buf_fromstring));
    table.set_str("tostring", Value::Native("buffer.tostring", buf_tostring));
    table.set_str("len", Value::Native("buffer.len", buf_len));
    table.set_str("readu8", Value::Native("buffer.readu8", buf_readu8));
    table.set_str("writeu8", Value::Native("buffer.writeu8", buf_writeu8));
    table.set_str("readi8", Value::Native("buffer.readi8", buf_readi8));
    table.set_str("writei8", Value::Native("buffer.writei8", buf_writei8));
    table.set_str("readu16", Value::Native("buffer.readu16", buf_readu16));
    table.set_str("writeu16", Value::Native("buffer.writeu16", buf_writeu16));
    table.set_str("readi16", Value::Native("buffer.readi16", buf_readi16));
    table.set_str("writei16", Value::Native("buffer.writei16", buf_writei16));
    table.set_str("readu32", Value::Native("buffer.readu32", buf_readu32));
    table.set_str("writeu32", Value::Native("buffer.writeu32", buf_writeu32));
    table.set_str("readi32", Value::Native("buffer.readi32", buf_readi32));
    table.set_str("writei32", Value::Native("buffer.writei32", buf_writei32));
    table.set_str("readf32", Value::Native("buffer.readf32", buf_readf32));
    table.set_str("writef32", Value::Native("buffer.writef32", buf_writef32));
    table.set_str("readf64", Value::Native("buffer.readf64", buf_readf64));
    table.set_str("writef64", Value::Native("buffer.writef64", buf_writef64));
    table.set_str("readstring", Value::Native("buffer.readstring", buf_readstring));
    table.set_str("writestring", Value::Native("buffer.writestring", buf_writestring));
    table.set_str("copy", Value::Native("buffer.copy", buf_copy));
    table.set_str("fill", Value::Native("buffer.fill", buf_fill));
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}
