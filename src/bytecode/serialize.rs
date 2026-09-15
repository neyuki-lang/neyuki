// Binary serialization and deserialization for Neyuki bytecode files.

use num_bigint::{BigInt, Sign};
use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::{Constant, Proto, UpvalueDesc};

// Magic bytes at the start of compiled bytecode binary
pub const MAGIC: &[u8; 7] = b"neyuki!";
pub const BYTECODE_VERSION: u8 = 1;

pub fn serialize(proto: &Proto) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.push(BYTECODE_VERSION);
    write_proto(&mut buf, proto);
    buf
}

pub fn deserialize(bytes: &[u8]) -> Result<Proto, String> {
    if bytes.len() < 8 {
        return Err("bytecode buffer too small".to_string());
    }
    if &bytes[0..7] != MAGIC {
        return Err("invalid bytecode magic bytes: expected 'neyuki!'".to_string());
    }
    let version = bytes[7];
    if version != BYTECODE_VERSION {
        return Err(format!("unsupported bytecode version: {}", version));
    }

    let mut cursor = 8usize;
    read_proto(bytes, &mut cursor)
}

fn write_u8(buf: &mut Vec<u8>, val: u8) {
    buf.push(val);
}

fn write_u16(buf: &mut Vec<u8>, val: u16) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_i16(buf: &mut Vec<u8>, val: i16) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_u32(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_i32(buf: &mut Vec<u8>, val: i32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_f64(buf: &mut Vec<u8>, val: f64) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

fn write_bigint(buf: &mut Vec<u8>, b: &BigInt) {
    let (sign, bytes) = b.to_bytes_le();
    let sign_byte = match sign {
        Sign::Minus => 0u8,
        Sign::NoSign => 1u8,
        Sign::Plus => 2u8,
    };
    write_u8(buf, sign_byte);
    write_u32(buf, bytes.len() as u32);
    buf.extend_from_slice(&bytes);
}

fn write_constant(buf: &mut Vec<u8>, c: &Constant) {
    match c {
        Constant::Nil => write_u8(buf, 0),
        Constant::Bool(b) => {
            write_u8(buf, 1);
            write_u8(buf, if *b { 1 } else { 0 });
        }
        Constant::Int(i) => {
            write_u8(buf, 2);
            write_bigint(buf, i);
        }
        Constant::Float(f) => {
            write_u8(buf, 3);
            write_f64(buf, *f);
        }
        Constant::String(s) => {
            write_u8(buf, 4);
            write_string(buf, s);
        }
    }
}

fn write_instruction(buf: &mut Vec<u8>, inst: &Instruction) {
    match inst {
        Instruction::LoadNil { dst } => {
            write_u8(buf, 1);
            write_u8(buf, *dst);
        }
        Instruction::LoadBool { dst, val } => {
            write_u8(buf, 2);
            write_u8(buf, *dst);
            write_u8(buf, if *val { 1 } else { 0 });
        }
        Instruction::LoadInt { dst, val } => {
            write_u8(buf, 3);
            write_u8(buf, *dst);
            write_i32(buf, *val);
        }
        Instruction::LoadK { dst, k } => {
            write_u8(buf, 4);
            write_u8(buf, *dst);
            write_u16(buf, *k);
        }
        Instruction::Move { dst, src } => {
            write_u8(buf, 5);
            write_u8(buf, *dst);
            write_u8(buf, *src);
        }
        Instruction::GetGlobal { dst, name_k } => {
            write_u8(buf, 6);
            write_u8(buf, *dst);
            write_u16(buf, *name_k);
        }
        Instruction::SetGlobal { src, name_k } => {
            write_u8(buf, 7);
            write_u8(buf, *src);
            write_u16(buf, *name_k);
        }
        Instruction::GetUpval { dst, upval_idx } => {
            write_u8(buf, 8);
            write_u8(buf, *dst);
            write_u8(buf, *upval_idx);
        }
        Instruction::SetUpval { src, upval_idx } => {
            write_u8(buf, 9);
            write_u8(buf, *src);
            write_u8(buf, *upval_idx);
        }
        Instruction::NewTable { dst } => {
            write_u8(buf, 10);
            write_u8(buf, *dst);
        }
        Instruction::GetTable { dst, table, key } => {
            write_u8(buf, 11);
            write_u8(buf, *dst);
            write_u8(buf, *table);
            write_u8(buf, *key);
        }
        Instruction::SetTable { table, key, val } => {
            write_u8(buf, 12);
            write_u8(buf, *table);
            write_u8(buf, *key);
            write_u8(buf, *val);
        }
        Instruction::GetTableK { dst, table, key_k } => {
            write_u8(buf, 13);
            write_u8(buf, *dst);
            write_u8(buf, *table);
            write_u16(buf, *key_k);
        }
        Instruction::SetTableK { table, key_k, val } => {
            write_u8(buf, 14);
            write_u8(buf, *table);
            write_u16(buf, *key_k);
            write_u8(buf, *val);
        }
        Instruction::AppendArray { table, src } => {
            write_u8(buf, 15);
            write_u8(buf, *table);
            write_u8(buf, *src);
        }
        Instruction::Add { dst, a, b } => {
            write_u8(buf, 16);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Sub { dst, a, b } => {
            write_u8(buf, 17);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Mul { dst, a, b } => {
            write_u8(buf, 18);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Div { dst, a, b } => {
            write_u8(buf, 19);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::IDiv { dst, a, b } => {
            write_u8(buf, 20);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Mod { dst, a, b } => {
            write_u8(buf, 21);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Pow { dst, a, b } => {
            write_u8(buf, 22);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::BitAnd { dst, a, b } => {
            write_u8(buf, 23);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::BitOr { dst, a, b } => {
            write_u8(buf, 24);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::BitXor { dst, a, b } => {
            write_u8(buf, 25);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Shl { dst, a, b } => {
            write_u8(buf, 26);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Shr { dst, a, b } => {
            write_u8(buf, 27);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Concat { dst, a, b } => {
            write_u8(buf, 28);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Unm { dst, src } => {
            write_u8(buf, 29);
            write_u8(buf, *dst);
            write_u8(buf, *src);
        }
        Instruction::Not { dst, src } => {
            write_u8(buf, 30);
            write_u8(buf, *dst);
            write_u8(buf, *src);
        }
        Instruction::Len { dst, src } => {
            write_u8(buf, 31);
            write_u8(buf, *dst);
            write_u8(buf, *src);
        }
        Instruction::BitNot { dst, src } => {
            write_u8(buf, 32);
            write_u8(buf, *dst);
            write_u8(buf, *src);
        }
        Instruction::Coalesce { dst, a, b } => {
            write_u8(buf, 33);
            write_u8(buf, *dst);
            write_u8(buf, *a);
            write_u8(buf, *b);
        }
        Instruction::Eq { a, b, jump_if_false } => {
            write_u8(buf, 34);
            write_u8(buf, *a);
            write_u8(buf, *b);
            write_i16(buf, *jump_if_false);
        }
        Instruction::Ne { a, b, jump_if_false } => {
            write_u8(buf, 35);
            write_u8(buf, *a);
            write_u8(buf, *b);
            write_i16(buf, *jump_if_false);
        }
        Instruction::Lt { a, b, jump_if_false } => {
            write_u8(buf, 36);
            write_u8(buf, *a);
            write_u8(buf, *b);
            write_i16(buf, *jump_if_false);
        }
        Instruction::Le { a, b, jump_if_false } => {
            write_u8(buf, 37);
            write_u8(buf, *a);
            write_u8(buf, *b);
            write_i16(buf, *jump_if_false);
        }
        Instruction::Gt { a, b, jump_if_false } => {
            write_u8(buf, 38);
            write_u8(buf, *a);
            write_u8(buf, *b);
            write_i16(buf, *jump_if_false);
        }
        Instruction::Ge { a, b, jump_if_false } => {
            write_u8(buf, 39);
            write_u8(buf, *a);
            write_u8(buf, *b);
            write_i16(buf, *jump_if_false);
        }
        Instruction::Test { reg, jump_if_false } => {
            write_u8(buf, 40);
            write_u8(buf, *reg);
            write_i16(buf, *jump_if_false);
        }
        Instruction::Jump { offset } => {
            write_u8(buf, 41);
            write_i16(buf, *offset);
        }
        Instruction::Call { callee, argc, retc } => {
            write_u8(buf, 42);
            write_u8(buf, *callee);
            write_u8(buf, *argc);
            write_u8(buf, *retc);
        }
        Instruction::Return { base, count } => {
            write_u8(buf, 43);
            write_u8(buf, *base);
            write_u8(buf, *count);
        }
        Instruction::Closure { dst, proto_idx } => {
            write_u8(buf, 44);
            write_u8(buf, *dst);
            write_u16(buf, *proto_idx);
        }
        Instruction::Vararg { dst, count } => {
            write_u8(buf, 45);
            write_u8(buf, *dst);
            write_u8(buf, *count);
        }
    }
}

fn write_proto(buf: &mut Vec<u8>, proto: &Proto) {
    if let Some(name) = &proto.name {
        write_u8(buf, 1);
        write_string(buf, name);
    } else {
        write_u8(buf, 0);
    }
    write_u8(buf, proto.num_params);
    write_u8(buf, proto.max_registers);
    write_u8(buf, if proto.is_vararg { 1 } else { 0 });

    // Write constants
    write_u32(buf, proto.constants.len() as u32);
    for c in &proto.constants {
        write_constant(buf, c);
    }

    // Write instructions
    write_u32(buf, proto.instructions.len() as u32);
    for inst in &proto.instructions {
        write_instruction(buf, inst);
    }

    // Write nested prototypes
    write_u32(buf, proto.protos.len() as u32);
    for child in &proto.protos {
        write_proto(buf, child);
    }

    // Write upvalues
    write_u32(buf, proto.upvalues.len() as u32);
    for upval in &proto.upvalues {
        write_u8(buf, if upval.in_stack { 1 } else { 0 });
        write_u8(buf, upval.index);
    }

    // Write line numbers
    write_u32(buf, proto.lines.len() as u32);
    for line in &proto.lines {
        write_u32(buf, *line);
    }
}

fn read_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, String> {
    if *cursor >= bytes.len() {
        return Err("unexpected end of bytecode".to_string());
    }
    let val = bytes[*cursor];
    *cursor += 1;
    Ok(val)
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, String> {
    if *cursor + 2 > bytes.len() {
        return Err("unexpected end of bytecode".to_string());
    }
    let val = u16::from_le_bytes([bytes[*cursor], bytes[*cursor + 1]]);
    *cursor += 2;
    Ok(val)
}

fn read_i16(bytes: &[u8], cursor: &mut usize) -> Result<i16, String> {
    if *cursor + 2 > bytes.len() {
        return Err("unexpected end of bytecode".to_string());
    }
    let val = i16::from_le_bytes([bytes[*cursor], bytes[*cursor + 1]]);
    *cursor += 2;
    Ok(val)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, String> {
    if *cursor + 4 > bytes.len() {
        return Err("unexpected end of bytecode".to_string());
    }
    let val = u32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(val)
}

fn read_i32(bytes: &[u8], cursor: &mut usize) -> Result<i32, String> {
    if *cursor + 4 > bytes.len() {
        return Err("unexpected end of bytecode".to_string());
    }
    let val = i32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(val)
}

fn read_f64(bytes: &[u8], cursor: &mut usize) -> Result<f64, String> {
    if *cursor + 8 > bytes.len() {
        return Err("unexpected end of bytecode".to_string());
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[*cursor..*cursor + 8]);
    *cursor += 8;
    Ok(f64::from_le_bytes(b))
}

fn read_string(bytes: &[u8], cursor: &mut usize) -> Result<String, String> {
    let len = read_u32(bytes, cursor)? as usize;
    if *cursor + len > bytes.len() {
        return Err("unexpected end of bytecode in string".to_string());
    }
    let s = std::str::from_utf8(&bytes[*cursor..*cursor + len])
        .map_err(|e| format!("invalid utf-8: {}", e))?
        .to_string();
    *cursor += len;
    Ok(s)
}

fn read_bigint(bytes: &[u8], cursor: &mut usize) -> Result<BigInt, String> {
    let sign_byte = read_u8(bytes, cursor)?;
    let sign = match sign_byte {
        0 => Sign::Minus,
        1 => Sign::NoSign,
        2 => Sign::Plus,
        _ => return Err("invalid bigint sign".to_string()),
    };
    let len = read_u32(bytes, cursor)? as usize;
    if *cursor + len > bytes.len() {
        return Err("unexpected end of bytecode in bigint".to_string());
    }
    let b = BigInt::from_bytes_le(sign, &bytes[*cursor..*cursor + len]);
    *cursor += len;
    Ok(b)
}

fn read_constant(bytes: &[u8], cursor: &mut usize) -> Result<Constant, String> {
    let tag = read_u8(bytes, cursor)?;
    match tag {
        0 => Ok(Constant::Nil),
        1 => {
            let b = read_u8(bytes, cursor)? != 0;
            Ok(Constant::Bool(b))
        }
        2 => {
            let bi = read_bigint(bytes, cursor)?;
            Ok(Constant::Int(bi))
        }
        3 => {
            let f = read_f64(bytes, cursor)?;
            Ok(Constant::Float(f))
        }
        4 => {
            let s = read_string(bytes, cursor)?;
            Ok(Constant::String(s))
        }
        _ => Err(format!("invalid constant tag: {}", tag)),
    }
}

fn read_instruction(bytes: &[u8], cursor: &mut usize) -> Result<Instruction, String> {
    let tag = read_u8(bytes, cursor)?;
    match tag {
        1 => Ok(Instruction::LoadNil { dst: read_u8(bytes, cursor)? }),
        2 => Ok(Instruction::LoadBool {
            dst: read_u8(bytes, cursor)?,
            val: read_u8(bytes, cursor)? != 0,
        }),
        3 => Ok(Instruction::LoadInt {
            dst: read_u8(bytes, cursor)?,
            val: read_i32(bytes, cursor)?,
        }),
        4 => Ok(Instruction::LoadK {
            dst: read_u8(bytes, cursor)?,
            k: read_u16(bytes, cursor)?,
        }),
        5 => Ok(Instruction::Move {
            dst: read_u8(bytes, cursor)?,
            src: read_u8(bytes, cursor)?,
        }),
        6 => Ok(Instruction::GetGlobal {
            dst: read_u8(bytes, cursor)?,
            name_k: read_u16(bytes, cursor)?,
        }),
        7 => Ok(Instruction::SetGlobal {
            src: read_u8(bytes, cursor)?,
            name_k: read_u16(bytes, cursor)?,
        }),
        8 => Ok(Instruction::GetUpval {
            dst: read_u8(bytes, cursor)?,
            upval_idx: read_u8(bytes, cursor)?,
        }),
        9 => Ok(Instruction::SetUpval {
            src: read_u8(bytes, cursor)?,
            upval_idx: read_u8(bytes, cursor)?,
        }),
        10 => Ok(Instruction::NewTable { dst: read_u8(bytes, cursor)? }),
        11 => Ok(Instruction::GetTable {
            dst: read_u8(bytes, cursor)?,
            table: read_u8(bytes, cursor)?,
            key: read_u8(bytes, cursor)?,
        }),
        12 => Ok(Instruction::SetTable {
            table: read_u8(bytes, cursor)?,
            key: read_u8(bytes, cursor)?,
            val: read_u8(bytes, cursor)?,
        }),
        13 => Ok(Instruction::GetTableK {
            dst: read_u8(bytes, cursor)?,
            table: read_u8(bytes, cursor)?,
            key_k: read_u16(bytes, cursor)?,
        }),
        14 => Ok(Instruction::SetTableK {
            table: read_u8(bytes, cursor)?,
            key_k: read_u16(bytes, cursor)?,
            val: read_u8(bytes, cursor)?,
        }),
        15 => Ok(Instruction::AppendArray {
            table: read_u8(bytes, cursor)?,
            src: read_u8(bytes, cursor)?,
        }),
        16 => Ok(Instruction::Add {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        17 => Ok(Instruction::Sub {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        18 => Ok(Instruction::Mul {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        19 => Ok(Instruction::Div {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        20 => Ok(Instruction::IDiv {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        21 => Ok(Instruction::Mod {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        22 => Ok(Instruction::Pow {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        23 => Ok(Instruction::BitAnd {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        24 => Ok(Instruction::BitOr {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        25 => Ok(Instruction::BitXor {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        26 => Ok(Instruction::Shl {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        27 => Ok(Instruction::Shr {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        28 => Ok(Instruction::Concat {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        29 => Ok(Instruction::Unm {
            dst: read_u8(bytes, cursor)?,
            src: read_u8(bytes, cursor)?,
        }),
        30 => Ok(Instruction::Not {
            dst: read_u8(bytes, cursor)?,
            src: read_u8(bytes, cursor)?,
        }),
        31 => Ok(Instruction::Len {
            dst: read_u8(bytes, cursor)?,
            src: read_u8(bytes, cursor)?,
        }),
        32 => Ok(Instruction::BitNot {
            dst: read_u8(bytes, cursor)?,
            src: read_u8(bytes, cursor)?,
        }),
        33 => Ok(Instruction::Coalesce {
            dst: read_u8(bytes, cursor)?,
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
        }),
        34 => Ok(Instruction::Eq {
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
            jump_if_false: read_i16(bytes, cursor)?,
        }),
        35 => Ok(Instruction::Ne {
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
            jump_if_false: read_i16(bytes, cursor)?,
        }),
        36 => Ok(Instruction::Lt {
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
            jump_if_false: read_i16(bytes, cursor)?,
        }),
        37 => Ok(Instruction::Le {
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
            jump_if_false: read_i16(bytes, cursor)?,
        }),
        38 => Ok(Instruction::Gt {
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
            jump_if_false: read_i16(bytes, cursor)?,
        }),
        39 => Ok(Instruction::Ge {
            a: read_u8(bytes, cursor)?,
            b: read_u8(bytes, cursor)?,
            jump_if_false: read_i16(bytes, cursor)?,
        }),
        40 => Ok(Instruction::Test {
            reg: read_u8(bytes, cursor)?,
            jump_if_false: read_i16(bytes, cursor)?,
        }),
        41 => Ok(Instruction::Jump { offset: read_i16(bytes, cursor)? }),
        42 => Ok(Instruction::Call {
            callee: read_u8(bytes, cursor)?,
            argc: read_u8(bytes, cursor)?,
            retc: read_u8(bytes, cursor)?,
        }),
        43 => Ok(Instruction::Return {
            base: read_u8(bytes, cursor)?,
            count: read_u8(bytes, cursor)?,
        }),
        44 => Ok(Instruction::Closure {
            dst: read_u8(bytes, cursor)?,
            proto_idx: read_u16(bytes, cursor)?,
        }),
        45 => Ok(Instruction::Vararg {
            dst: read_u8(bytes, cursor)?,
            count: read_u8(bytes, cursor)?,
        }),
        _ => Err(format!("unknown instruction tag: {}", tag)),
    }
}

fn read_proto(bytes: &[u8], cursor: &mut usize) -> Result<Proto, String> {
    let has_name = read_u8(bytes, cursor)? != 0;
    let name = if has_name {
        Some(read_string(bytes, cursor)?)
    } else {
        None
    };
    let num_params = read_u8(bytes, cursor)?;
    let max_registers = read_u8(bytes, cursor)?;
    let is_vararg = read_u8(bytes, cursor)? != 0;

    let num_constants = read_u32(bytes, cursor)? as usize;
    let mut constants = Vec::with_capacity(num_constants);
    for _ in 0..num_constants {
        constants.push(read_constant(bytes, cursor)?);
    }

    let num_instructions = read_u32(bytes, cursor)? as usize;
    let mut instructions = Vec::with_capacity(num_instructions);
    for _ in 0..num_instructions {
        instructions.push(read_instruction(bytes, cursor)?);
    }

    let num_protos = read_u32(bytes, cursor)? as usize;
    let mut protos = Vec::with_capacity(num_protos);
    for _ in 0..num_protos {
        protos.push(read_proto(bytes, cursor)?);
    }

    let num_upvalues = read_u32(bytes, cursor)? as usize;
    let mut upvalues = Vec::with_capacity(num_upvalues);
    for _ in 0..num_upvalues {
        let in_stack = read_u8(bytes, cursor)? != 0;
        let index = read_u8(bytes, cursor)?;
        upvalues.push(UpvalueDesc { in_stack, index });
    }

    let num_lines = read_u32(bytes, cursor)? as usize;
    let mut lines = Vec::with_capacity(num_lines);
    for _ in 0..num_lines {
        lines.push(read_u32(bytes, cursor)?);
    }

    Ok(Proto {
        name,
        num_params,
        max_registers,
        is_vararg,
        constants,
        instructions,
        protos,
        upvalues,
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_header_and_version() {
        let proto = Proto::new(Some("test".to_string()), 0, false);
        let bytes = serialize(&proto);
        assert_eq!(&bytes[0..7], b"neyuki!");
        assert_eq!(bytes[7], BYTECODE_VERSION);
    }

    #[test]
    fn test_rejects_invalid_magic() {
        let fake = b"badhead\x01\x00\x00";
        assert!(deserialize(fake).is_err());
    }

    #[test]
    fn test_proto_roundtrip() {
        let mut proto = Proto::new(Some("math_fn".to_string()), 2, false);
        proto.max_registers = 4;
        let k0 = proto.add_constant(Constant::Int(BigInt::from(42)));
        let k1 = proto.add_constant(Constant::String("hello".to_string()));
        proto.emit(Instruction::LoadK { dst: 0, k: k0 }, 10);
        proto.emit(Instruction::LoadK { dst: 1, k: k1 }, 11);
        proto.emit(Instruction::Add { dst: 2, a: 0, b: 1 }, 12);
        proto.emit(Instruction::Return { base: 2, count: 1 }, 13);

        let bytes = serialize(&proto);
        let decoded = deserialize(&bytes).expect("deserialize failed");

        assert_eq!(decoded.name, proto.name);
        assert_eq!(decoded.num_params, proto.num_params);
        assert_eq!(decoded.max_registers, proto.max_registers);
        assert_eq!(decoded.constants, proto.constants);
        assert_eq!(decoded.instructions, proto.instructions);
        assert_eq!(decoded.lines, proto.lines);
    }
}
