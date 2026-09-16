use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::bytecode::instruction::Instruction;
use crate::bytecode::proto::{Constant, Proto};
use crate::vm::frame::CallFrame;
use crate::vm::gc::GcTracker;
use crate::vm::libs::{
    create_bit_lib, create_buffer_lib, create_coroutine_lib, create_debug_lib, create_json_lib,
    create_math_lib, create_os_lib, create_string_lib, create_table_lib, create_utf8_lib,
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
        crate::vm::builtins::register_all(self);

        self.globals.insert("bit".to_string(), create_bit_lib());
        self.globals.insert("bit32".to_string(), create_bit_lib());
        self.globals.insert("buffer".to_string(), create_buffer_lib());
        self.globals.insert("math".to_string(), create_math_lib());
        self.globals.insert("string".to_string(), create_string_lib());
        self.globals.insert("table".to_string(), create_table_lib());
        self.globals.insert("os".to_string(), create_os_lib());
        self.globals.insert("coroutine".to_string(), create_coroutine_lib());
        self.globals.insert("debug".to_string(), create_debug_lib());
        self.globals.insert("json".to_string(), create_json_lib());
        self.globals.insert("utf8".to_string(), create_utf8_lib());
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

    pub fn call_function(&mut self, func: Value, args: &[Value]) -> Result<Vec<Value>, String> {
        const MAX_CALL_DEPTH: usize = 512;
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Err(format!(
                "call stack overflow: exceeded maximum call depth of {}",
                MAX_CALL_DEPTH
            ));
        }
        match func {
            Value::Native(_, f) => f(self, args),
            Value::Closure(c) => {
                let depth = self.frames.len();
                let base = self.stack.len() + 1;
                let needed = base + c.proto.max_registers as usize + args.len() + 1;
                if needed >= self.stack.len() {
                    self.stack.resize(needed + 1, Value::Nil);
                }
                for (i, arg) in args.iter().enumerate() {
                    self.stack[base + i] = arg.clone();
                }
                self.frames.push(CallFrame::new(c.clone(), base));
                match self.run_to_depth(depth) {
                    Ok(res) => Ok(vec![res]),
                    Err(err) => {
                        self.frames.truncate(depth);
                        Err(err)
                    }
                }
            }
            Value::Table(ref t) => {
                let mt_opt = t.borrow().metatable.clone();
                if let Some(mt) = mt_opt {
                    let call_handler = mt.borrow().fields.get("__call").cloned().unwrap_or(Value::Nil);
                    if !matches!(call_handler, Value::Nil) {
                        let mut call_args = vec![func.clone()];
                        call_args.extend_from_slice(args);
                        return self.call_function(call_handler, &call_args);
                    }
                }
                Err("attempt to call a table value without __call metamethod".to_string())
            }
            _ => Err("attempt to call a non-function value".to_string()),
        }
    }

    fn get_binop_metamethod(&self, a: &Value, b: &Value, event: &str) -> Option<Value> {
        if let Value::Table(t) = a
            && let Some(mt) = &t.borrow().metatable
                && let Some(h) = mt.borrow().fields.get(event).cloned()
                    && !matches!(h, Value::Nil) {
                        return Some(h);
                    }
        if let Value::Table(t) = b
            && let Some(mt) = &t.borrow().metatable
                && let Some(h) = mt.borrow().fields.get(event).cloned()
                    && !matches!(h, Value::Nil) {
                        return Some(h);
                    }
        None
    }

    fn get_unop_metamethod(&self, val: &Value, event: &str) -> Option<Value> {
        if let Value::Table(t) = val
            && let Some(mt) = &t.borrow().metatable
                && let Some(h) = mt.borrow().fields.get(event).cloned()
                    && !matches!(h, Value::Nil) {
                        return Some(h);
                    }
        None
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
                    let rc = Rc::new(RefCell::new(VmTable::new()));
                    self.gc.register_table(&rc);
                    let t = Value::Table(rc);
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
                Instruction::SetList { table, base, count } => {
                    let tbl = self.get_reg(table);
                    if let Value::Table(t) = tbl {
                        let mut t_mut = t.borrow_mut();
                        for i in 0..count {
                            let val = self.get_reg(base + i);
                            t_mut.array.push(val);
                        }
                    } else {
                        return Err("cannot SetList to non-table".to_string());
                    }
                }
                Instruction::TForCall { base, retc } => {
                    let iter_fn = self.get_reg(base);
                    let state = self.get_reg(base + 1);
                    let ctrl = self.get_reg(base + 2);
                    let results = self.call_function(iter_fn, &[state, ctrl])?;
                    let var_base = base + 3;
                    for i in 0..(retc as usize) {
                        let val = results.get(i).cloned().unwrap_or(Value::Nil);
                        self.set_reg(var_base + i as u8, val);
                    }
                }
                Instruction::TForLoop { base, jump } => {
                    let first_var = self.get_reg(base + 3);
                    if matches!(first_var, Value::Nil) {
                        // Exit loop: jump forward past the back-jump
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump as isize) as usize;
                    } else {
                        // Update ctrl to first result, continue
                        self.set_reg(base + 2, first_var);
                    }
                }
                Instruction::Add { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__add") {
                        let res = self.call_function(mm, &[va, vb])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let res = crate::vm::ops::eval_add(va, vb)?;
                        self.set_reg(dst, res);
                    }
                }
                Instruction::Sub { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__sub") {
                        let res = self.call_function(mm, &[va, vb])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let res = crate::vm::ops::eval_sub(va, vb)?;
                        self.set_reg(dst, res);
                    }
                }
                Instruction::Mul { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__mul") {
                        let res = self.call_function(mm, &[va, vb])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let res = crate::vm::ops::eval_mul(va, vb)?;
                        self.set_reg(dst, res);
                    }
                }
                Instruction::Div { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__div") {
                        let res = self.call_function(mm, &[va, vb])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let res = crate::vm::ops::eval_div(va, vb)?;
                        self.set_reg(dst, res);
                    }
                }
                Instruction::IDiv { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__idiv") {
                        let res = self.call_function(mm, &[va, vb])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let res = crate::vm::ops::eval_idiv(va, vb)?;
                        self.set_reg(dst, res);
                    }
                }
                Instruction::Mod { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__mod") {
                        let res = self.call_function(mm, &[va, vb])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let res = crate::vm::ops::eval_mod(va, vb)?;
                        self.set_reg(dst, res);
                    }
                }
                Instruction::Pow { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__pow") {
                        let res = self.call_function(mm, &[va, vb])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let res = crate::vm::ops::eval_pow(va, vb)?;
                        self.set_reg(dst, res);
                    }
                }
                Instruction::BitAnd { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = crate::vm::ops::eval_bitand(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::BitOr { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = crate::vm::ops::eval_bitor(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::BitXor { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = crate::vm::ops::eval_bitxor(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Shl { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = crate::vm::ops::eval_shl(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Shr { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = crate::vm::ops::eval_shr(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::LShl { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = crate::vm::ops::eval_lshl(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::LShr { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let res = crate::vm::ops::eval_lshr(va, vb)?;
                    self.set_reg(dst, res);
                }
                Instruction::Concat { dst, a, b } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__concat") {
                        let res = self.call_function(mm, &[va, vb])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let s = format!("{}{}", va, vb);
                        self.set_reg(dst, Value::String(s));
                    }
                }
                Instruction::Unm { dst, src } => {
                    let v = self.get_reg(src);
                    if let Some(mm) = self.get_unop_metamethod(&v, "__unm") {
                        let res = self.call_function(mm, &[v])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let res = match v {
                            Value::Int(i) => Value::Int(-i),
                            Value::Float(f) => Value::Float(-f),
                            _ => return Err("unary minus expects a number".to_string()),
                        };
                        self.set_reg(dst, res);
                    }
                }
                Instruction::Not { dst, src } => {
                    let v = self.get_reg(src);
                    self.set_reg(dst, Value::Bool(!v.is_truthy()));
                }
                Instruction::Len { dst, src } => {
                    let v = self.get_reg(src);
                    if let Some(mm) = self.get_unop_metamethod(&v, "__len") {
                        let res = self.call_function(mm, &[v])?;
                        self.set_reg(dst, res.first().cloned().unwrap_or(Value::Nil));
                    } else {
                        let len = match v {
                            Value::String(s) => BigInt::from(s.len()),
                            Value::Table(t) => BigInt::from(t.borrow().array.len()),
                            Value::Buffer(b) => BigInt::from(b.borrow().len()),
                            _ => return Err("len expects string, table or buffer".to_string()),
                        };
                        self.set_reg(dst, Value::Int(len));
                    }
                }
                Instruction::BitNot { dst, src } => {
                    let v = self.get_reg(src);
                    let bi = crate::vm::ops::to_bigint(v)?;
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
                    let is_eq = if va == vb {
                        true
                    } else if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__eq") {
                        let res = self.call_function(mm, &[va, vb])?;
                        res.first().map(|v| v.is_truthy()).unwrap_or(false)
                    } else {
                        false
                    };
                    if !is_eq {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Ne { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let is_eq = if va == vb {
                        true
                    } else if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__eq") {
                        let res = self.call_function(mm, &[va, vb])?;
                        res.first().map(|v| v.is_truthy()).unwrap_or(false)
                    } else {
                        false
                    };
                    if is_eq {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Lt { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let is_lt = if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__lt") {
                        let res = self.call_function(mm, &[va, vb])?;
                        res.first().map(|v| v.is_truthy()).unwrap_or(false)
                    } else {
                        crate::vm::ops::eval_lt(&va, &vb)?
                    };
                    if !is_lt {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Le { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let is_le = if let Some(mm) = self.get_binop_metamethod(&va, &vb, "__le") {
                        let res = self.call_function(mm, &[va, vb])?;
                        res.first().map(|v| v.is_truthy()).unwrap_or(false)
                    } else {
                        crate::vm::ops::eval_le(&va, &vb)?
                    };
                    if !is_le {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Gt { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let is_gt = if let Some(mm) = self.get_binop_metamethod(&vb, &va, "__lt") {
                        let res = self.call_function(mm, &[vb, va])?;
                        res.first().map(|v| v.is_truthy()).unwrap_or(false)
                    } else {
                        crate::vm::ops::eval_gt(&va, &vb)?
                    };
                    if !is_gt {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump_if_false as isize) as usize;
                    }
                }
                Instruction::Ge { a, b, jump_if_false } => {
                    let va = self.get_reg(a);
                    let vb = self.get_reg(b);
                    let is_ge = if let Some(mm) = self.get_binop_metamethod(&vb, &va, "__le") {
                        let res = self.call_function(mm, &[vb, va])?;
                        res.first().map(|v| v.is_truthy()).unwrap_or(false)
                    } else {
                        crate::vm::ops::eval_ge(&va, &vb)?
                    };
                    if !is_ge {
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
                Instruction::ForPrep { base, jump } => {
                    let init = self.get_reg(base);
                    let step = self.get_reg(base + 2);
                    let init_minus_step = crate::vm::ops::eval_sub(init, step)?;
                    self.set_reg(base, init_minus_step);
                    let frame = self.frames.last_mut().unwrap();
                    frame.ip = (frame.ip as isize + jump as isize) as usize;
                }
                Instruction::ForLoop { base, jump } => {
                    let idx = self.get_reg(base);
                    let limit = self.get_reg(base + 1);
                    let step = self.get_reg(base + 2);
                    let next_idx = crate::vm::ops::eval_add(idx, step.clone())?;
                    self.set_reg(base, next_idx.clone());
                    let is_positive = match &step {
                        Value::Int(i) => i >= &BigInt::zero(),
                        Value::Float(f) => *f >= 0.0,
                        _ => true,
                    };
                    let loop_again = if is_positive {
                        crate::vm::ops::eval_le(&next_idx, &limit)?
                    } else {
                        crate::vm::ops::eval_ge(&next_idx, &limit)?
                    };
                    if loop_again {
                        self.set_reg(base + 3, next_idx);
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip = (frame.ip as isize + jump as isize) as usize;
                    }
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
                            const MAX_CALL_DEPTH: usize = 512;
                            if self.frames.len() >= MAX_CALL_DEPTH {
                                return Err(format!(
                                    "call stack overflow: exceeded maximum call depth of {}",
                                    MAX_CALL_DEPTH
                                ));
                            }
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
                        Value::Table(ref t) => {
                            let mt_opt = t.borrow().metatable.clone();
                            let call_handler = mt_opt.as_ref().and_then(|m| m.borrow().fields.get("__call").cloned());
                            if let Some(handler) = call_handler {
                                let mut call_args = vec![callee_val.clone()];
                                call_args.extend_from_slice(&self.stack[args_start..args_end]);
                                let results = self.call_function(handler, &call_args)?;
                                let count = if retc == 0 { 1 } else { retc as usize };
                                for i in 0..count {
                                    let val = results.get(i).cloned().unwrap_or(Value::Nil);
                                    self.set_reg(callee + i as u8, val);
                                }
                            } else {
                                return Err(format!("attempted to call non-function ({:?})", callee_val));
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


    fn table_get(&mut self, table: &Value, key: &Value) -> Result<Value, String> {
        match table {
            Value::Table(t) => {
                let direct_val = {
                    let tbl = t.borrow();
                    match key {
                        Value::String(k) => tbl.fields.get(k).cloned(),
                        Value::Int(idx) if *idx > BigInt::zero() => {
                            let i = idx.to_usize().ok_or_else(|| "table index too large".to_string())?;
                            tbl.array.get(i - 1).cloned()
                        }
                        _ => None,
                    }
                };

                if let Some(val) = direct_val
                    && !matches!(val, Value::Nil) {
                        return Ok(val);
                    }

                // Check __index metamethod
                let mt_opt = t.borrow().metatable.clone();
                if let Some(mt) = mt_opt {
                    let index_handler = mt.borrow().fields.get("__index").cloned().unwrap_or(Value::Nil);
                    match index_handler {
                        Value::Table(_) => {
                            return self.table_get(&index_handler, key);
                        }
                        Value::Native(_, _) | Value::Closure(_) => {
                            let res = self.call_function(index_handler, &[table.clone(), key.clone()])?;
                            return Ok(res.into_iter().next().unwrap_or(Value::Nil));
                        }
                        _ => {}
                    }
                }
                Ok(Value::Nil)
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

    fn table_set(&mut self, table: &Value, key: Value, val: Value) -> Result<(), String> {
        self.gc.write_barrier(table, &val);
        match table {
            Value::Table(t) => {
                if t.borrow().frozen {
                    return Err("attempted to mutate a frozen table".to_string());
                }

                let key_exists = match &key {
                    Value::String(k) => t.borrow().fields.contains_key(k),
                    Value::Int(idx) if *idx > BigInt::zero() => {
                        let i = idx.to_usize().ok_or_else(|| "table index too large".to_string())?;
                        i - 1 < t.borrow().array.len()
                    }
                    _ => false,
                };

                if !key_exists {
                    let mt_opt = t.borrow().metatable.clone();
                    if let Some(mt) = mt_opt {
                        let newindex_handler = mt.borrow().fields.get("__newindex").cloned().unwrap_or(Value::Nil);
                        match newindex_handler {
                            Value::Table(_) => {
                                return self.table_set(&newindex_handler, key, val);
                            }
                            Value::Native(_, _) | Value::Closure(_) => {
                                self.call_function(newindex_handler, &[table.clone(), key, val])?;
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                }

                let mut tbl = t.borrow_mut();
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
                        } else {
                            tbl.array.resize(i - 1, Value::Nil);
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
    fn test_vm_logical_shifts() {
        let res = run_code("local a = -1 >>> 60\nlocal b = 1 <<< 4\nreturn a + b");
        // 15 + 16 = 31
        assert_eq!(res.to_string(), "31");
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

    #[test]
    fn test_vm_for_prep_and_for_loop() {
        let code = "local sum = 0\nfor i = 1, 5 do\n  sum = sum + i\nend\nreturn sum";
        let res = run_code(code);
        assert_eq!(res.to_string(), "15");

        let code_step = "local sum = 0\nfor i = 10, 2, -2 do\n  sum = sum + i\nend\nreturn sum";
        // 10 + 8 + 6 + 4 + 2 = 30
        let res_step = run_code(code_step);
        assert_eq!(res_step.to_string(), "30");
    }

    #[test]
    fn test_vm_setlist_table_construct() {
        let code = "local t = {10, 20, 30, 40, 50}\nreturn t[1] + t[3] + t[5]";
        // 10 + 30 + 50 = 90
        let res = run_code(code);
        assert_eq!(res.to_string(), "90");
    }

    #[test]
    fn test_vm_metatables_index_newindex() {
        let code = "local proto = { answer = 42 }\nlocal t = {}\nsetmetatable(t, { __index = proto })\nreturn t.answer";
        let res = run_code(code);
        assert_eq!(res.to_string(), "42");

        let code_fn = "local t = {}\nsetmetatable(t, { __index = function(tbl, key) return key .. \"_handled\" end })\nreturn t.foo";
        let res_fn = run_code(code_fn);
        assert_eq!(res_fn.to_string(), "foo_handled");

        let code_newindex = "local sink = {}\nlocal t = {}\nsetmetatable(t, { __newindex = sink })\nt.val = 123\nreturn sink.val";
        let res_newindex = run_code(code_newindex);
        assert_eq!(res_newindex.to_string(), "123");
    }

    #[test]
    fn test_vm_metatables_operators_and_call() {
        let code_add = "local mt = { __add = function(a, b) return a.val + b.val end }\nlocal a = { val = 15 }\nlocal b = { val = 27 }\nsetmetatable(a, mt)\nreturn a + b";
        let res_add = run_code(code_add);
        assert_eq!(res_add.to_string(), "42");

        let code_call = "local t = {}\nsetmetatable(t, { __call = function(tbl, x, y) return x * y end })\nreturn t(6, 7)";
        let res_call = run_code(code_call);
        assert_eq!(res_call.to_string(), "42");

        let code_tostring = "local t = { name = \"Neyuki\" }\nsetmetatable(t, { __tostring = function(x) return \"Hello \" .. x.name end })\nreturn tostring(t)";
        let res_tostring = run_code(code_tostring);
        assert_eq!(res_tostring.to_string(), "Hello Neyuki");
    }

    #[test]
    fn test_vm_raw_builtins() {
        let code = "local t = {}\nrawset(t, \"k\", 99)\nlocal a = rawget(t, \"k\")\nlocal eq = rawequal(t, t)\nassert(eq == true)\nreturn a";
        let res = run_code(code);
        assert_eq!(res.to_string(), "99");
    }

    #[test]
    fn test_vm_require_bundled_libs() {
        let code_bit = "local bit = require(\"@neyuki/bit\")\nreturn bit.band(7, 3)";
        let res_bit = run_code(code_bit);
        assert_eq!(res_bit.to_string(), "3");

        let code_buf = "local buffer = require(\"@neyuki/buffer\")\nlocal b = buffer.create(8)\nbuffer.writeu8(b, 0, 99)\nreturn buffer.readu8(b, 0)";
        let res_buf = run_code(code_buf);
        assert_eq!(res_buf.to_string(), "99");

        let code_os = "local os = require(\"@neyuki/os\")\nlocal t = os.clock()\nassert(t >= 0)\nreturn 1";
        let res_os = run_code(code_os);
        assert_eq!(res_os.to_string(), "1");
    }

    #[test]
    fn test_vm_utf8_library() {
        let code = "local u = require(\"@neyuki/utf8\")\nlocal s = u.char(65, 66, 67)\nassert(u.len(s) == 3)\nassert(u.codepoint(s, 1, 1) == 65)\nreturn s";
        let res = run_code(code);
        assert_eq!(res.to_string(), "ABC");
    }

    #[test]
    fn test_vm_json_library() {
        let code = "local j = require(\"@neyuki/json\")\nlocal s = j.encode({ name = \"Neyuki\", version = 1 })\nlocal obj = j.decode(s)\nreturn obj.name";
        let res = run_code(code);
        assert_eq!(res.to_string(), "Neyuki");
    }

    #[test]
    fn test_vm_debug_library() {
        let code = "local d = require(\"@neyuki/debug\")\nlocal trace = d.traceback(\"test trace\")\nassert(string.find(trace, \"stack traceback\") != nil)\nreturn 1";
        let res = run_code(code);
        assert_eq!(res.to_string(), "1");
    }

    #[test]
    fn test_vm_coroutine_library() {
        let code = "local co = require(\"@neyuki/coroutine\")\nlocal f = function(x) return x * 10 end\nlocal t = co.create(f)\nlocal ok, val = co.resume(t, 5)\nassert(ok == true)\nassert(val == 50)\nassert(co.status(t) == \"dead\")\nreturn val";
        let res = run_code(code);
        assert_eq!(res.to_string(), "50");
    }

    #[test]
    fn test_vm_call_stack_overflow_caught() {
        // Infinite recursion via Instruction::Call must be caught by MAX_CALL_DEPTH = 512
        let code = "local function f()\n  return f()\nend\nreturn f()";
        let stmts = crate::compiler::compile_source(code).expect("syntax error");
        let proto = crate::compiler::compile_to_proto(&stmts);
        let mut vm = VM::new();
        let res = vm.execute(proto);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("call stack overflow"));
    }

    #[test]
    fn test_vm_string_rep_limits() {
        // Empty string with giant n should be caught without allocating GBs of memory
        let code = "return string.rep(\"\", 2000000000)";
        let stmts = crate::compiler::compile_source(code).expect("syntax error");
        let proto = crate::compiler::compile_to_proto(&stmts);
        let mut vm = VM::new();
        let res = vm.execute(proto);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("exceeds maximum limit"));
    }
}

