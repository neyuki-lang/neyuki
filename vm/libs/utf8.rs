// UTF-8 standard library for Neyuki VM.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

pub fn create_utf8_lib() -> Value {
    let t = Rc::new(RefCell::new(VmTable::new()));
    let mut b = t.borrow_mut();

    b.set_str("char", crate::native!("utf8.char", utf8_char));
    b.set_str(
        "codepoint",
        crate::native!("utf8.codepoint", utf8_codepoint),
    );
    b.set_str("len", crate::native!("utf8.len", utf8_len));
    b.set_str("offset", crate::native!("utf8.offset", utf8_offset));
    b.set_str("codes", crate::native!("utf8.codes", utf8_codes));
    b.set_str(
        "charpattern",
        Value::str("[\u{0000}-\u{007F}\u{00C2}-\u{00FD}][\u{0080}-\u{00BF}]*"),
    );
    drop(b);

    Value::Table(t)
}

fn utf8_char(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut out = String::new();
    for (idx, arg) in args.iter().enumerate() {
        let code = match arg {
            Value::Int(i) => i.to_u32().ok_or_else(|| {
                format!(
                    "bad argument #{} to 'utf8.char' (value out of range)",
                    idx + 1
                )
            })?,
            Value::Float(f) => *f as u32,
            _ => {
                return Err(format!(
                    "bad argument #{} to 'utf8.char' (number expected, got {})",
                    idx + 1,
                    arg.type_name()
                ));
            }
        };

        if code > 0x10FFFF || (0xD800..=0xDFFF).contains(&code) {
            return Err(format!(
                "bad argument #{} to 'utf8.char' (value out of range)",
                idx + 1
            ));
        }

        if let Some(c) = char::from_u32(code) {
            out.push(c);
        } else {
            return Err(format!(
                "bad argument #{} to 'utf8.char' (invalid codepoint)",
                idx + 1
            ));
        }
    }
    Ok(vec![Value::string(out)])
}

fn utf8_codepoint(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = match args.first() {
        Some(Value::String(s)) => &**s,
        _ => return Err("bad argument #1 to 'utf8.codepoint' (string expected)".to_string()),
    };

    let len = s.len() as isize;
    let i = match args.get(1) {
        Some(Value::Int(n)) => n.to_isize().unwrap_or(1),
        Some(Value::Float(f)) => *f as isize,
        _ => 1,
    };
    let j = match args.get(2) {
        Some(Value::Int(n)) => n.to_isize().unwrap_or(i),
        Some(Value::Float(f)) => *f as isize,
        _ => i,
    };

    let start_byte = if i > 0 {
        (i - 1).min(len) as usize
    } else {
        (len + i).max(0) as usize
    };

    let end_byte = if j > 0 {
        j.min(len) as usize
    } else {
        (len + j + 1).max(0) as usize
    };

    if start_byte >= end_byte || start_byte >= s.len() {
        return Ok(Vec::new());
    }

    let slice = match s.get(start_byte..end_byte) {
        Some(sl) => sl,
        None => return Err("invalid UTF-8 byte boundary in slice".to_string()),
    };

    let mut results = Vec::new();
    for ch in slice.chars() {
        results.push(Value::from_bigint(BigInt::from(ch as u32)));
    }

    Ok(results)
}

fn utf8_len(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = match args.first() {
        Some(Value::String(s)) => &**s,
        _ => return Err("bad argument #1 to 'utf8.len' (string expected)".to_string()),
    };

    let len = s.len() as isize;
    let i = match args.get(1) {
        Some(Value::Int(n)) => n.to_isize().unwrap_or(1),
        Some(Value::Float(f)) => *f as isize,
        _ => 1,
    };
    let j = match args.get(2) {
        Some(Value::Int(n)) => n.to_isize().unwrap_or(-1),
        Some(Value::Float(f)) => *f as isize,
        _ => -1,
    };

    let start_byte = if i > 0 {
        (i - 1).min(len) as usize
    } else {
        (len + i).max(0) as usize
    };

    let end_byte = if j > 0 {
        j.min(len) as usize
    } else {
        (len + j + 1).max(0) as usize
    };

    if start_byte > end_byte || start_byte > s.len() {
        return Ok(vec![Value::from_bigint(BigInt::from(0))]);
    }

    let bytes = s.as_bytes();
    let slice = &bytes[start_byte..end_byte.min(bytes.len())];

    match std::str::from_utf8(slice) {
        Ok(valid_str) => Ok(vec![Value::from_bigint(BigInt::from(
            valid_str.chars().count(),
        ))]),
        Err(err) => {
            let error_pos = start_byte + err.valid_up_to() + 1;
            Ok(vec![
                Value::Nil,
                Value::from_bigint(BigInt::from(error_pos)),
            ])
        }
    }
}

fn utf8_offset(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = match args.first() {
        Some(Value::String(s)) => &**s,
        _ => return Err("bad argument #1 to 'utf8.offset' (string expected)".to_string()),
    };

    let n = match args.get(1) {
        Some(Value::Int(i)) => i.to_isize().unwrap_or(1),
        Some(Value::Float(f)) => *f as isize,
        _ => return Err("bad argument #2 to 'utf8.offset' (number expected)".to_string()),
    };

    let i = match args.get(2) {
        Some(Value::Int(v)) => {
            v.to_isize()
                .unwrap_or(if n >= 0 { 1 } else { s.len() as isize + 1 })
        }
        Some(Value::Float(f)) => *f as isize,
        _ => {
            if n >= 0 {
                1
            } else {
                s.len() as isize + 1
            }
        }
    };

    let len = s.len() as isize;
    let start_byte = if i > 0 {
        (i - 1).min(len) as usize
    } else {
        (len + i).max(0) as usize
    };

    if start_byte > s.len() {
        return Ok(vec![Value::Nil]);
    }

    if n == 0 {
        // Find beginning of current char
        let mut pos = start_byte;
        while pos > 0 && !s.is_char_boundary(pos) {
            pos -= 1;
        }
        return Ok(vec![Value::from_bigint(BigInt::from(pos + 1))]);
    }

    let mut count = 0isize;
    if n > 0 {
        for (idx, _) in s[start_byte..].char_indices() {
            count += 1;
            if count == n {
                return Ok(vec![Value::from_bigint(BigInt::from(start_byte + idx + 1))]);
            }
        }
    } else {
        // n < 0
        let target = -n;
        let mut indices: Vec<usize> = Vec::new();
        for (idx, _) in s[..start_byte].char_indices() {
            indices.push(idx);
        }
        if (target as usize) <= indices.len() {
            let pos = indices[indices.len() - target as usize];
            return Ok(vec![Value::from_bigint(BigInt::from(pos + 1))]);
        }
    }

    Ok(vec![Value::Nil])
}

/// One step of the `utf8.codes` iterator: the state table carries the
/// string and the byte position of the next character.
fn utf8_iter(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let state = match args.first() {
        Some(Value::Table(t)) => t.clone(),
        _ => return Ok(vec![Value::Nil]),
    };

    let (str_val, pos_val) = {
        let b = state.borrow();
        (b.get_str("string"), b.get_str("pos"))
    };

    let s = match str_val {
        Value::String(s) => s,
        _ => return Ok(vec![Value::Nil]),
    };

    let pos = match pos_val {
        Value::Int(i) => i.to_usize().unwrap_or(1),
        _ => 1,
    };

    let start_byte = pos.saturating_sub(1);
    if start_byte >= s.len() {
        return Ok(vec![Value::Nil]);
    }

    if let Some(ch) = s[start_byte..].chars().next() {
        let next_pos = pos + ch.len_utf8();
        state
            .borrow_mut()
            .set_str("pos", Value::from_bigint(BigInt::from(next_pos)));
        Ok(vec![
            Value::from_bigint(BigInt::from(pos)),
            Value::from_bigint(BigInt::from(ch as u32)),
        ])
    } else {
        Ok(vec![Value::Nil])
    }
}

fn utf8_codes(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = match args.first() {
        Some(Value::String(s)) => s.clone(),
        _ => return Err("bad argument #1 to 'utf8.codes' (string expected)".to_string()),
    };

    // Return a stateful table iterator or closure
    let tbl = Rc::new(RefCell::new(VmTable::new()));
    tbl.borrow_mut().set_str("string", Value::String(s));
    tbl.borrow_mut()
        .set_str("pos", Value::from_bigint(BigInt::from(1)));

    let iter_fn = crate::native!("utf8_iter", utf8_iter);

    Ok(vec![iter_fn, Value::Table(tbl)])
}
