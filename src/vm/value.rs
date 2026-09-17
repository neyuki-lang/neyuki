// VM runtime values and table structures.

use num_bigint::BigInt;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::bytecode::proto::Proto;
use crate::vm::buffer::VmBuffer;

pub type NativeFn = fn(&mut crate::vm::machine::VM, &[Value]) -> Result<Vec<Value>, String>;

#[derive(Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    String(String),
    Table(Rc<RefCell<VmTable>>),
    Closure(Rc<VmClosure>),
    Native(&'static str, NativeFn),
    Buffer(Rc<RefCell<VmBuffer>>),
}

pub struct VmTable {
    pub array: Vec<Value>,
    pub fields: HashMap<String, Value>,
    pub frozen: bool,
    pub metatable: Option<Rc<RefCell<VmTable>>>,
}

impl VmTable {
    pub fn new() -> Self {
        Self {
            array: Vec::new(),
            fields: HashMap::new(),
            frozen: false,
            metatable: None,
        }
    }

    #[allow(dead_code)]
    pub fn get_str(&self, key: &str) -> Value {
        self.fields.get(key).cloned().unwrap_or(Value::Nil)
    }

    pub fn set_str(&mut self, key: &str, val: Value) {
        if !self.frozen {
            self.fields.insert(key.to_string(), val);
        }
    }
}

pub struct VmClosure {
    pub proto: Proto,
    pub upvalues: Vec<Rc<RefCell<Value>>>,
}

impl Value {
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Nil => false,
            Value::Bool(b) => *b,
            _ => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Int(_) | Value::Float(_) => "number",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Closure(_) | Value::Native(..) => "function",
            Value::Buffer(_) => "buffer",
        }
    }

    pub fn typeof_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Int(_) => "bigint",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Closure(_) | Value::Native(..) => "function",
            Value::Buffer(_) => "buffer",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "{}", s),
            Value::Table(_) => write!(f, "table"),
            Value::Closure(c) => write!(
                f,
                "function({})",
                c.proto.name.as_deref().unwrap_or("anonymous")
            ),
            Value::Native(name, _) => write!(f, "native function({})", name),
            Value::Buffer(b) => write!(f, "buffer({})", b.borrow().len()),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => {
                num_traits::ToPrimitive::to_f64(a).is_some_and(|v| v == *b)
            }
            (Value::Float(a), Value::Int(b)) => {
                num_traits::ToPrimitive::to_f64(b).is_some_and(|v| *a == v)
            }
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Table(a), Value::Table(b)) => Rc::ptr_eq(a, b),
            (Value::Closure(a), Value::Closure(b)) => Rc::ptr_eq(a, b),
            (Value::Native(n1, _), Value::Native(n2, _)) => n1 == n2,
            (Value::Buffer(a), Value::Buffer(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}
