// Register-based Virtual Machine execution engine.

use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{FromPrimitive, ToPrimitive, Zero};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;

use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::{Constant, Proto};
use crate::vm::frame::CallFrame;
use crate::vm::gc::GcTracker;
use crate::vm::libs::{
    create_bit_lib, create_buffer_lib, create_math_lib, create_os_lib, create_string_lib,
    create_table_lib,
};
use crate::vm::value::{NativeFn, Value, VmClosure, VmTable};

pub struct VM {
    pub stack: Vec<Value>,
    pub frames: Vec<CallFrame>,
    pub globals: HashMap<String, Value>,
    pub gc: GcTracker,
}

impl VM {
    pub fn new() -> Self {
        let mut vm = Self {
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(64),
            globals: HashMap::new(),
            gc: GcTracker::new(),
        };
        vm.register_builtins();
        vm
    }

    fn register_builtins(&mut self) {
        self.register_native("print", builtin_print);
        self.register_native("assert", builtin_assert);
        self.register_native("type", builtin_type);
        self.register_native("typeof", builtin_typeof);
        self.register_native("tostring", builtin_tostring);
        self.register_native("tonumber", builtin_tonumber);
        self.register_native("int", builtin_int);
        self.register_native("float", builtin_float);
        self.register_native("collectgarbage", builtin_collectgarbage);
        self.register_native("error", builtin_error);
        self.register_native("pcall", builtin_pcall);
        self.register_native("xpcall", builtin_xpcall);
        self.register_native("require", builtin_require);

        self.globals.insert("bit".to_string(), create_bit_lib());
        self.globals.insert("bit32".to_string(), create_bit_lib());
        self.globals.insert("buffer".to_string(), create_buffer_lib());
        self.globals.insert("math".to_string(), create_math_lib());
        self.globals.insert("string".to_string(), create_string_lib());
        self.globals.insert("table".to_string(), create_table_lib());
        self.globals.insert("os".to_string(), create_os_lib());
    }

    pub fn register_native(&mut self, name: &'static str, func: NativeFn) {
        self.globals.insert(name.to_string(), Value::Native(name, func));
    }

    pub fn execute(&mut self, proto: Proto) -> Result<Value, String> {
        let closure = Rc::new(VmClosure {
            proto,
            upvalues: Vec::new(),
        });
        self.stack.clear();
        self.frames.clear();

        // Ensure stack has enough capacity for main frame
        let max_reg = closure.proto.max_registers as usize;
        self.stack.resize(max_reg + 1, Value::Nil);

        self.frames.push(CallFrame::new(closure, 0));
        self.run()
    }

    fn get_reg(&self, reg: u8) -> Value {
        let base = self.frames.last().unwrap().base;
        self.stack.get(base + reg as usize).cloned().unwrap_or(Value::Nil)
    }

    fn set_reg(&mut self, reg: u8, val: Value) {
        let base = self.frames.last().unwrap().base;
        let idx = base + reg as usize;
        if idx >= self.stack.len() {
            self.stack.resize(idx + 1, Value::Nil);
        }
        self.stack[idx] = val;
    }

    fn get_constant(&self, k: u16) -> Value {
        let frame = self.frames.last().unwrap();
        match &frame.closure.proto.constants[k as usize] {
            Constant::Nil => Value::Nil,
            Constant::Bool(b) => Value::Bool(*b),
            Constant::Int(i) => Value::Int(i.clone()),
            Constant::Float(f) => Value::Float(*f),
            Constant::String(s) => Value::String(s.clone()),
        }
    }

    pub fn run(&mut self) -> Result<Value, String> {
        self.run_to_depth(0)
    }

    pub fn run_to_depth(&mut self, target_depth: usize) -> Result<Value, String> {
        while self.frames.len() > target_depth {
            let frame = self.frames.last_mut().unwrap();
            if frame.ip >= frame.closure.proto.instructions.len() {
                self.frames.pop();
                continue;
            }

            let inst = frame.closure.proto.instructions[frame.ip].clone();
            frame.ip += 1;

            match inst {
                Instruction::LoadNil { dst } => {
                    self.set_reg(dst, Value::Nil);
                }
                Instruction::LoadBool { dst, val } => {
                    self.set_reg(dst, Value::Bool(val));
                }
                Instruction::LoadInt { dst, val } => {
                    self.set_reg(dst, Value::Int(BigInt::from(val)));
                }
                Instruction::LoadK { dst, k } => {
                    let val = self.get_constant(k);
                    self.set_reg(dst, val);
                }
                Instruction::Move { dst, src } => {
                    let val = self.get_reg(src);
                    self.set_reg(dst, val);
                }
                Instruction::GetGlobal { dst, name_k } => {
                    let name = match self.get_constant(name_k) {
                        Value::String(s) => s,
                        _ => return Err("global name must be string".to_string()),
                    };
                    let val = self.globals.get(&name).cloned().unwrap_or(Value::Nil);
                    self.set_reg(dst, val);
                }
                Instruction::SetGlobal { src, name_k } => {
                    let name = match self.get_constant(name_k) {
                        Value::String(s) => s,
                        _ => return Err("global name must be string".to_string()),
                    };
                    let val = self.get_reg(src);
                    self.globals.insert(name, val);
                }
                Instruction::GetUpval { dst, upval_idx } => {
                    let frame = self.frames.last().unwrap();
                    let val = frame.closure.upvalues[upval_idx as usize].borrow().clone();
                    self.set_reg(dst, val);
                }
                Instruction::SetUpval { src, upval_idx } => {
                    let val = self.get_reg(src);
                    let frame = self.frames.last().unwrap();
                    *frame.closure.upvalues[upval_idx as usize].borrow_mut() = val;
                }
                Instruction::NewTable { dst } => {
                    let t = Value::Table(Rc::new(RefCell::new(VmTable::new())));
                    self.set_reg(dst, t);
                }
                Instruction::GetTable { dst, table, key } => {
                    let tbl = self.get_reg(table);
                    let k = self.get_reg(key);
                    let val = self.table_get(&tbl, &k)?;
                    self.set_reg(dst, val);
                }
                Instruction::SetTable { table, key, val } => {
                    let tbl = self.get_reg(table);
                    let k = self.get_reg(key);
                    let v = self.get_reg(val);
                    self.table_set(&tbl, k, v)?;
                }
                Instruction::GetTableK { dst, table, key_k } => {
                    let tbl = self.get_reg(table);
                    let k = self.get_constant(key_k);
                    let val = self.table_get(&tbl, &k)?;
                    self.set_reg(dst, val);
                }
                Instruction::SetTableK { table, key_k, val } => {
                    let tbl = self.get_reg(table);
                    let k = self.get_constant(key_k);
                    let v = self.get_reg(val);
                    self.table_set(&tbl, k, v)?;
                }
                Instruction::AppendArray { table, src } => {
                    let tbl = self.get_reg(table);
                    let val = self.get_reg(src);
                    if let Value::Table(t) = tbl {
                        t.borrow_mut().array.push(val);
                    } else {
                        return Err("cannot append to non-table".to_string());
                    }
                }
                Instruction::Add { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_add(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Sub { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_sub(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Mul { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_mul(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Div { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_div(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::IDiv { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_idiv(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Mod { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_mod(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Pow { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_pow(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::BitAnd { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_bitand(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::BitOr { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_bitor(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::BitXor { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_bitxor(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Shl { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_shl(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Shr { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = self.eval_shr(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Concat { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let s = format!("{}{}", va, vb);
                    self.set_reg(dst, Value::String(s));
                }
                Instruction::Unm { dst, src } => {
                    let v = self.get_reg(src);
                    let res = match v {
                        Value::Int(i) => Value::Int(-i),
                        Value::Float(f) => Value::Float(-f),
                        _ => return Err("unary minus expects a number".to_string()),
                    };
                    self.set_reg(dst, res);
                }
                Instruction::Not { dst, src } => {
                    let v = self.get_reg(src);
                    self.set_reg(dst, Value::Bool(!v.is_truthy()));
                }
                Instruction::Len { dst, src } => {
                    let v = self.get_reg(src);
                    let len = match v {
                        Value::String(s) => BigInt::from(s.len()),
                        Value::Table(t) => BigInt::from(t.borrow().array.len()),
                        Value::Buffer(b) => BigInt::from(b.borrow().len()),
                        _ => return Err("len expects string, table or buffer".to_string()),
                    };
                    self.set_reg(dst, Value::Int(len));
                }
                Instruction::BitNot { dst, src } => {
                    let v = self.get_reg(src);
                    let bi = self.to_bigint(v)?;
                    self.set_reg(dst, Value::Int(!bi));
                }
                Instruction::Coalesce { dst, a, b } => {
                    let va = self.get_reg(a);
                    if matches!(va, Value::Nil) {
                        let vb = self.get_reg(b);
                        self.set_reg(dst, vb);
                    } else {
                        self.set_reg(dst, va);
                    }
                }
                Instruction::Eq { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if va != vb {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Ne { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if va == vb {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Lt { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if !self.eval_lt(&va, &vb)? {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Le { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if !self.eval_le(&va, &vb)? {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Gt { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if !self.eval_gt(&va, &vb)? {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Ge { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if !self.eval_ge(&va, &vb)? {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Test { reg, jump_if_false } => {
                    let val = self.get_reg(reg);
                    if !val.is_truthy() {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Jump { offset } => {
                    let frame = self.frames.last_mut().unwrap();
                    frame.ip = (frame.ip as isize + offset as isize) as usize;
                }
                Instruction::Closure { dst, proto_idx } => {
                    let child_proto = {
                        let frame = self.frames.last().unwrap();
                        frame.closure.proto.protos[proto_idx as usize].clone()
                    };
                    let mut upvalues = Vec::new();
                    for updesc in &child_proto.upvalues {
                        if updesc.in_stack {
                            let base = self.frames.last().unwrap().base;
                            let reg_val = self.stack[base + updesc.index as usize].clone();
                            upvalues.push(Rc::new(RefCell::new(reg_val)));
                        } else {
                            let frame = self.frames.last().unwrap();
                            let up = frame.closure.upvalues[updesc.index as usize].clone();
                            upvalues.push(up);
                        }
                    }
                    let closure = Value::Closure(Rc::new(VmClosure {
                        proto: child_proto,
                        upvalues,
                    }));
                    self.set_reg(dst, closure);
                }
                Instruction::Call { callee, argc, retc } => {
                    let callee_val = self.get_reg(callee);
                    let base = self.frames.last().unwrap().base;
                    let args_start = base + callee as usize + 1;
                    let args_end = args_start + argc as usize;

                    match callee_val {
                        Value::Closure(closure) => {
                            let new_base = base + callee as usize + 1;
                            let needed = new_base + closure.proto.max_registers as usize;
                            if needed >= self.stack.len() {
                                self.stack.resize(needed + 1, Value::Nil);
                            }
                            let new_frame = CallFrame::new(closure, new_base);
                            self.frames.push(new_frame);
                        }
                        Value::Native(_, func) => {
                            let args = self.stack[args_start..args_end].to_vec();
                            let results = func(self, &args)?;
                            let count = if retc == 0 { 1 } else { retc as usize };
                            for i in 0..count {
                                let val = results.get(i).cloned().unwrap_or(Value::Nil);
                                self.set_reg(callee + i as u8, val);
                            }
                        }
                        _ => return Err(format!("attempted to call non-function ({:?})", callee_val)),
                    }
                }
                Instruction::Return { base, count } => {
                    let frame = self.frames.pop().unwrap();
                    let return_count = count as usize;
                    let mut ret_vals = Vec::with_capacity(return_count);
                    for i in 0..return_count {
                        ret_vals.push(self.stack[frame.base + base as usize + i].clone());
                    }
                    let top_val = ret_vals.first().cloned().unwrap_or(Value::Nil);
                    if self.frames.len() == target_depth {
                        return Ok(top_val);
                    }
                    let caller_dest = frame.base - 1;
                    for (i, val) in ret_vals.into_iter().enumerate() {
                        let idx = caller_dest + i;
                        if idx >= self.stack.len() {
                            self.stack.resize(idx + 1, Value::Nil);
                        }
                        self.stack[idx] = val;
                    }
                }
                Instruction::Vararg { dst, count: _ } => {
                    self.set_reg(dst, Value::Nil);
                }
            }
        }

        Ok(Value::Nil)
    }

    fn to_bigint(&self, v: Value) -> Result<BigInt, String> {
        match v {
            Value::Int(i) => Ok(i),
            Value::Float(f) => BigInt::from_f64(f.trunc()).ok_or_else(|| "cannot convert float to integer".to_string()),
            _ => Err("expected integer".to_string()),
        }
    }

    fn to_f64(&self, v: Value) -> Result<f64, String> {
        match v {
            Value::Float(f) => Ok(f),
            Value::Int(i) => i.to_f64().ok_or_else(|| "integer overflow in float conversion".to_string()),
            _ => Err("expected number".to_string()),
        }
    }

    fn eval_add(&self, a: Value, b: Value) -> Result<Value, String> {
        if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
            return Ok(Value::Int(ia + ib));
        }
        let fa = self.to_f64(a)?;
        let fb = self.to_f64(b)?;
        Ok(Value::Float(fa + fb))
    }

    fn eval_sub(&self, a: Value, b: Value) -> Result<Value, String> {
        if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
            return Ok(Value::Int(ia - ib));
        }
        let fa = self.to_f64(a)?;
        let fb = self.to_f64(b)?;
        Ok(Value::Float(fa - fb))
    }

    fn eval_mul(&self, a: Value, b: Value) -> Result<Value, String> {
        if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
            return Ok(Value::Int(ia * ib));
        }
        let fa = self.to_f64(a)?;
        let fb = self.to_f64(b)?;
        Ok(Value::Float(fa * fb))
    }

    fn eval_div(&self, a: Value, b: Value) -> Result<Value, String> {
        let fa = self.to_f64(a)?;
        let fb = self.to_f64(b)?;
        if fb == 0.0 {
            return Err("division by zero".to_string());
        }
        Ok(Value::Float(fa / fb))
    }

    fn eval_idiv(&self, a: Value, b: Value) -> Result<Value, String> {
        if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
            if ib.is_zero() {
                return Err("division by zero".to_string());
            }
            return Ok(Value::Int(ia.div_floor(ib)));
        }
        let fa = self.to_f64(a)?;
        let fb = self.to_f64(b)?;
        if fb == 0.0 {
            return Err("division by zero".to_string());
        }
        Ok(Value::Float((fa / fb).floor()))
    }

    fn eval_mod(&self, a: Value, b: Value) -> Result<Value, String> {
        if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
            if ib.is_zero() {
                return Err("modulo by zero".to_string());
            }
            return Ok(Value::Int(ia.mod_floor(ib)));
        }
        let fa = self.to_f64(a)?;
        let fb = self.to_f64(b)?;
        if fb == 0.0 {
            return Err("modulo by zero".to_string());
        }
        Ok(Value::Float(fa % fb))
    }

    fn eval_pow(&self, a: Value, b: Value) -> Result<Value, String> {
        if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
            if ib.sign() != Sign::Minus {
                if let Some(exp) = ib.to_u32() {
                    return Ok(Value::Int(ia.pow(exp)));
                }
            }
        }
        let fa = self.to_f64(a)?;
        let fb = self.to_f64(b)?;
        Ok(Value::Float(fa.powf(fb)))
    }

    fn eval_bitand(&self, a: Value, b: Value) -> Result<Value, String> {
        let ia = self.to_bigint(a)?;
        let ib = self.to_bigint(b)?;
        Ok(Value::Int(ia & ib))
    }

    fn eval_bitor(&self, a: Value, b: Value) -> Result<Value, String> {
        let ia = self.to_bigint(a)?;
        let ib = self.to_bigint(b)?;
        Ok(Value::Int(ia | ib))
    }

    fn eval_bitxor(&self, a: Value, b: Value) -> Result<Value, String> {
        let ia = self.to_bigint(a)?;
        let ib = self.to_bigint(b)?;
        Ok(Value::Int(ia ^ ib))
    }

    fn eval_shl(&self, a: Value, b: Value) -> Result<Value, String> {
        let ia = self.to_bigint(a)?;
        let ib = self.to_bigint(b)?;
        if ib.sign() == Sign::Minus {
            let shift = (-ib).to_usize().ok_or_else(|| "shift is too large".to_string())?;
            Ok(Value::Int(ia >> shift))
        } else {
            let shift = ib.to_usize().ok_or_else(|| "shift is too large".to_string())?;
            Ok(Value::Int(ia << shift))
        }
    }

    fn eval_shr(&self, a: Value, b: Value) -> Result<Value, String> {
        let ia = self.to_bigint(a)?;
        let ib = self.to_bigint(b)?;
        if ib.sign() == Sign::Minus {
            let shift = (-ib).to_usize().ok_or_else(|| "shift is too large".to_string())?;
            Ok(Value::Int(ia << shift))
        } else {
            let shift = ib.to_usize().ok_or_else(|| "shift is too large".to_string())?;
            Ok(Value::Int(ia >> shift))
        }
    }

    fn eval_lt(&self, a: &Value, b: &Value) -> Result<bool, String> {
        match (a, b) {
            (Value::Int(ia), Value::Int(ib)) => Ok(ia < ib),
            (Value::String(sa), Value::String(sb)) => Ok(sa < sb),
            _ => Ok(self.to_f64(a.clone())? < self.to_f64(b.clone())?),
        }
    }

    fn eval_le(&self, a: &Value, b: &Value) -> Result<bool, String> {
        match (a, b) {
            (Value::Int(ia), Value::Int(ib)) => Ok(ia <= ib),
            (Value::String(sa), Value::String(sb)) => Ok(sa <= sb),
            _ => Ok(self.to_f64(a.clone())? <= self.to_f64(b.clone())?),
        }
    }

    fn eval_gt(&self, a: &Value, b: &Value) -> Result<bool, String> {
        match (a, b) {
            (Value::Int(ia), Value::Int(ib)) => Ok(ia > ib),
            (Value::String(sa), Value::String(sb)) => Ok(sa > sb),
            _ => Ok(self.to_f64(a.clone())? > self.to_f64(b.clone())?),
        }
    }

    fn eval_ge(&self, a: &Value, b: &Value) -> Result<bool, String> {
        match (a, b) {
            (Value::Int(ia), Value::Int(ib)) => Ok(ia >= ib),
            (Value::String(sa), Value::String(sb)) => Ok(sa >= sb),
            _ => Ok(self.to_f64(a.clone())? >= self.to_f64(b.clone())?),
        }
    }

    fn table_get(&self, table: &Value, key: &Value) -> Result<Value, String> {
        match table {
            Value::Table(t) => {
                let tbl = t.borrow();
                match key {
                    Value::String(k) => Ok(tbl.fields.get(k).cloned().unwrap_or(Value::Nil)),
                    Value::Int(idx) if *idx > BigInt::zero() => {
                        let i = idx.to_usize().ok_or_else(|| "table index too large".to_string())?;
                        Ok(tbl.array.get(i - 1).cloned().unwrap_or(Value::Nil))
                    }
                    _ => Ok(Value::Nil),
                }
            }
            Value::Buffer(b) => {
                let buf = b.borrow();
                match key {
                    Value::Int(idx) => {
                        let i = idx.to_usize().ok_or_else(|| "buffer index too large".to_string())?;
                        let val = buf.read_u8(i)?;
                        Ok(Value::Int(BigInt::from(val)))
                    }
                    _ => Ok(Value::Nil),
                }
            }
            _ => Err("attempted to index a non-table value".to_string()),
        }
    }

    fn table_set(&self, table: &Value, key: Value, val: Value) -> Result<(), String> {
        match table {
            Value::Table(t) => {
                let mut tbl = t.borrow_mut();
                if tbl.frozen {
                    return Err("attempted to mutate a frozen table".to_string());
                }
                match key {
                    Value::String(k) => {
                        tbl.fields.insert(k, val);
                    }
                    Value::Int(idx) if idx > BigInt::zero() => {
                        let i = idx.to_usize().ok_or_else(|| "table index too large".to_string())?;
                        if i - 1 < tbl.array.len() {
                            tbl.array[i - 1] = val;
                        } else if i - 1 == tbl.array.len() {
                            tbl.array.push(val);
                        }
                    }
                    _ => return Err("invalid table key".to_string()),
                }
                Ok(())
            }
            Value::Buffer(b) => {
                let mut buf = b.borrow_mut();
                match key {
                    Value::Int(idx) => {
                        let i = idx.to_usize().ok_or_else(|| "buffer index too large".to_string())?;
                        let byte_val = match val {
                            Value::Int(v) => v.to_u8().ok_or_else(|| "value out of byte range".to_string())?,
                            Value::Float(f) => f as u8,
                            _ => return Err("buffer value expects byte".to_string()),
                        };
                        buf.write_u8(i, byte_val)?;
                        Ok(())
                    }
                    _ => Err("invalid buffer index".to_string()),
                }
            }
            _ => Err("attempted to set field on a non-table value".to_string()),
        }
    }
}

fn builtin_print(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let out = args
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join("\t");
    println!("{}", out);
    Ok(vec![Value::Nil])
}

fn builtin_assert(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let cond = args.first().cloned().unwrap_or(Value::Nil);
    if !cond.is_truthy() {
        let msg = args.get(1).map(|v| v.to_string()).unwrap_or_else(|| "assertion failed".to_string());
        return Err(msg);
    }
    Ok(vec![cond])
}

fn builtin_type(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    Ok(vec![Value::String(val.type_name().to_string())])
}

fn builtin_typeof(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    Ok(vec![Value::String(val.typeof_name().to_string())])
}

fn builtin_tostring(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    Ok(vec![Value::String(val.to_string())])
}

fn builtin_tonumber(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    match val {
        Value::Int(i) => Ok(vec![Value::Int(i.clone())]),
        Value::Float(f) => Ok(vec![Value::Float(*f)]),
        Value::String(s) => {
            if let Ok(i) = s.parse::<i64>() {
                Ok(vec![Value::Int(BigInt::from(i))])
            } else if let Ok(bi) = BigInt::from_str(s) {
                Ok(vec![Value::Int(bi)])
            } else if let Ok(f) = s.parse::<f64>() {
                Ok(vec![Value::Float(f)])
            } else {
                Ok(vec![Value::Nil])
            }
        }
        _ => Ok(vec![Value::Nil]),
    }
}

fn builtin_int(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    match val {
        Value::Int(i) => Ok(vec![Value::Int(i.clone())]),
        Value::Float(f) => Ok(vec![Value::Int(BigInt::from_f64(f.trunc()).unwrap_or_default())]),
        Value::String(s) => {
            let bi = BigInt::parse_bytes(s.as_bytes(), 10).ok_or_else(|| "invalid integer string".to_string())?;
            Ok(vec![Value::Int(bi)])
        }
        _ => Err("int expects number or string".to_string()),
    }
}

fn builtin_float(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    match val {
        Value::Float(f) => Ok(vec![Value::Float(*f)]),
        Value::Int(i) => Ok(vec![Value::Float(i.to_f64().unwrap_or(0.0))]),
        Value::String(s) => {
            let f = s.parse::<f64>().map_err(|_| "invalid float string".to_string())?;
            Ok(vec![Value::Float(f)])
        }
        _ => Err("float expects number or string".to_string()),
    }
}

fn builtin_collectgarbage(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let opt = args.first().map(|v| v.to_string()).unwrap_or_else(|| "collect".to_string());
    match opt.as_str() {
        "collect" => {
            vm.gc.collect();
            Ok(vec![Value::Int(BigInt::zero())])
        }
        "stop" => {
            vm.gc.stop();
            Ok(vec![Value::Nil])
        }
        "restart" => {
            vm.gc.restart();
            Ok(vec![Value::Nil])
        }
        "count" => {
            Ok(vec![Value::Float(vm.gc.count_kb())])
        }
        "isrunning" => {
            Ok(vec![Value::Bool(vm.gc.is_running)])
        }
        "step" => {
            let step_size = args.get(1).and_then(|v| match v {
                Value::Int(i) => i.to_usize(),
                _ => None,
            }).unwrap_or(1024);
            let collected = vm.gc.step(step_size);
            Ok(vec![Value::Bool(collected)])
        }
        _ => Err(format!("unknown collectgarbage option '{}'", opt)),
    }
}

fn builtin_error(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let msg = args.first().map(|v| v.to_string()).unwrap_or_else(|| "error".to_string());
    Err(msg)
}

fn builtin_pcall(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let func = args.first().ok_or_else(|| "pcall expects at least 1 argument".to_string())?;
    match func {
        Value::Native(_, f) => {
            match f(vm, &args[1..]) {
                Ok(mut res) => {
                    res.insert(0, Value::Bool(true));
                    Ok(res)
                }
                Err(err) => Ok(vec![Value::Bool(false), Value::String(err)]),
            }
        }
        Value::Closure(c) => {
            let depth = vm.frames.len();
            let base = vm.stack.len() + 1;
            let needed = base + c.proto.max_registers as usize + args.len();
            vm.stack.resize(needed, Value::Nil);
            for (i, arg) in args[1..].iter().enumerate() {
                vm.stack[base + i] = arg.clone();
            }
            vm.frames.push(CallFrame::new(c.clone(), base));
            match vm.run_to_depth(depth) {
                Ok(res) => Ok(vec![Value::Bool(true), res]),
                Err(err) => {
                    vm.frames.truncate(depth);
                    Ok(vec![Value::Bool(false), Value::String(err)])
                }
            }
        }
        _ => Err("attempt to pcall non-function".to_string()),
    }
}

fn builtin_xpcall(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let func = args.first().ok_or_else(|| "xpcall expects at least 2 arguments".to_string())?;
    let err_handler = args.get(1).ok_or_else(|| "xpcall expects at least 2 arguments".to_string())?;
    let pcall_res = builtin_pcall(vm, &[func.clone()])?;
    if pcall_res[0] == Value::Bool(true) {
        Ok(pcall_res)
    } else {
        let err_msg = pcall_res.get(1).cloned().unwrap_or(Value::Nil);
        match err_handler {
            Value::Native(_, f) => {
                let h_res = f(vm, &[err_msg])?;
                let ret = h_res.into_iter().next().unwrap_or(Value::Nil);
                Ok(vec![Value::Bool(false), ret])
            }
            _ => Ok(vec![Value::Bool(false), err_msg]),
        }
    }
}

fn builtin_require(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let pkg = match args.first().ok_or_else(|| "require expects a module path".to_string())? {
        Value::String(s) => s.as_str(),
        _ => return Err("require expects string argument".to_string()),
    };

    let mod_name = match pkg {
        "@neyuki/math" | "math" => "math",
        "@neyuki/table" | "table" => "table",
        "@neyuki/string" | "string" => "string",
        "@neyuki/bit" | "bit" | "@neyuki/bit32" | "bit32" => "bit",
        "@neyuki/buffer" | "buffer" => "buffer",
        "@neyuki/os" | "os" => "os",
        other => {
            let path = if other.ends_with(".nyk") || other.ends_with(".nykb") {
                other.to_string()
            } else {
                format!("{}.nyk", other)
            };
            if let Ok(src) = std::fs::read_to_string(&path) {
                let stmts = crate::compiler::compile_source(&src)?;
                let proto = crate::compiler::compile_to_proto(&stmts);
                let val = vm.execute(proto)?;
                return Ok(vec![val]);
            }
            return Err(format!("cannot find module '{}'", pkg));
        }
    };

    if let Some(val) = vm.globals.get(mod_name).cloned() {
        Ok(vec![val])
    } else {
        Err(format!("module '{}' not found", mod_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile_to_proto;
    use crate::parser::Parser;

    fn run_code(src: &str) -> Value {
        let mut parser = Parser::new(src);
        let stmts = parser.parse_program();
        let proto = compile_to_proto(&stmts);
        let mut vm = VM::new();
        vm.execute(proto).expect("execution error")
    }

    #[test]
    fn test_vm_arithmetic() {
        let res = run_code("local a = 10\nlocal b = 20\nreturn a + b * 2");
        assert_eq!(res.to_string(), "50");
    }

    #[test]
    fn test_vm_bitwise_operations() {
        let res = run_code("local a = 12 & 10\nlocal b = 12 | 10\nlocal c = 12 ~ 10\nreturn a + b + c");
        // 8 + 14 + 6 = 28
        assert_eq!(res.to_string(), "28");
    }

    #[test]
    fn test_vm_bitwise_not_and_shifts() {
        let res = run_code("local a = ~0\nlocal b = 1 << 4\nlocal c = 16 >> 2\nreturn a + b + c");
        // -1 + 16 + 4 = 19
        assert_eq!(res.to_string(), "19");
    }

    #[test]
    fn test_vm_functions_and_calls() {
        let code = "function add(x, y) return x + y end\nreturn add(15, 27)";
        let res = run_code(code);
        assert_eq!(res.to_string(), "42");
    }

    #[test]
    fn test_vm_if_and_loops() {
        let code = "local sum = 0\nlocal i = 1\nwhile i <= 5 do\n  sum = sum + i\n  i++\nend\nreturn sum";
        let res = run_code(code);
        assert_eq!(res.to_string(), "15");
    }

    #[test]
    fn test_vm_tables() {
        let code = "local t = {10, 20, 30}\nt[2] = 99\nreturn t[2]";
        let res = run_code(code);
        assert_eq!(res.to_string(), "99");
    }

    #[test]
    fn test_vm_bit_library() {
        let code = "local band = bit.band(15, 7)\nlocal bor = bit.bor(8, 2)\nlocal bxor = bit.bxor(10, 6)\nreturn band + bor + bxor";
        // 7 + 10 + 12 = 29
        let res = run_code(code);
        assert_eq!(res.to_string(), "29");
    }

    #[test]
    fn test_vm_buffer_library() {
        let code = "local b = buffer.create(16)\nbuffer.writeu8(b, 0, 42)\nbuffer.writei32(b, 4, 123456)\nlocal u = buffer.readu8(b, 0)\nlocal i = buffer.readi32(b, 4)\nreturn u + i";
        // 42 + 123456 = 123498
        let res = run_code(code);
        assert_eq!(res.to_string(), "123498");
    }

    #[test]
    fn test_vm_gc_library() {
        let code = "local kb = collectgarbage(\"count\")\nassert(kb >= 0)\nlocal running = collectgarbage(\"isrunning\")\nassert(running == true)\nreturn 1";
        let res = run_code(code);
        assert_eq!(res.to_string(), "1");
    }

    #[test]
    fn test_vm_math_library() {
        let code = "local a = math.abs(-15)\nlocal b = math.floor(3.9)\nlocal c = math.max(10, 25, 5)\nreturn a + b + c";
        // 15 + 3 + 25 = 43
        let res = run_code(code);
        assert_eq!(res.to_string(), "43");
    }

    #[test]
    fn test_vm_string_library() {
        let code = "local s = \"hello\"\nlocal upper = string.upper(s)\nlocal sub = string.sub(s, 1, 4)\nreturn upper .. \"_\" .. sub";
        let res = run_code(code);
        assert_eq!(res.to_string(), "HELLO_hell");
    }

    #[test]
    fn test_vm_table_library() {
        let code = "local t = {10, 20}\ntable.insert(t, 30)\nlocal s = table.concat(t, \",\")\nreturn s";
        let res = run_code(code);
        assert_eq!(res.to_string(), "10,20,30");
    }

    #[test]
    fn test_vm_pcall_and_error() {
        let code = "local ok, err = pcall(error, \"boom\")\nassert(ok == false)\nassert(err == \"boom\")\nreturn 1";
        let res = run_code(code);
        assert_eq!(res.to_string(), "1");
    }

    #[test]
    fn test_vm_and_or_short_circuit() {
        let code = "local a = false and 10\nlocal b = 5 or 20\nlocal c = false or 30\nlocal d = true and 40\nreturn b + c + d";
        // 5 + 30 + 40 = 75
        let res = run_code(code);
        assert_eq!(res.to_string(), "75");
    }
}

