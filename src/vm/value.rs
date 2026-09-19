// VM runtime values and table structures.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use crate::bytecode::proto::Proto;
use crate::vm::buffer::VmBuffer;
use crate::vm::hash::{FxHashMap, new_map};

pub type NativeFn = fn(&mut crate::vm::machine::VM, &[Value]) -> Result<Vec<Value>, String>;

/// A native function together with its name. Values hold these by static
/// reference, which keeps `Value` at three words.
pub struct NativeDef {
    pub name: &'static str,
    pub func: NativeFn,
}

/// A `&'static NativeDef` for a name and function known at compile time.
#[macro_export]
macro_rules! native_def {
    ($name:expr, $func:expr) => {{
        const DEF: $crate::vm::value::NativeDef = $crate::vm::value::NativeDef {
            name: $name,
            func: $func,
        };
        &DEF
    }};
}

/// A `Value::Native` for a name and function known at compile time.
#[macro_export]
macro_rules! native {
    ($name:expr, $func:expr) => {
        $crate::vm::value::Value::Native($crate::native_def!($name, $func))
    };
}

/// A runtime value. Kept small and cheap to clone: every heap-backed variant
/// is behind an `Rc`, so copying a register never copies a string or a table.
///
/// Integers are arbitrary precision as far as programs can tell, but almost
/// all of them fit an `i64`. `Int` is the fast path; arithmetic that
/// overflows promotes to `BigInt`, and every constructor normalises so that a
/// `BigInt` is never inside the `i64` range. That invariant is what lets
/// `Int == BigInt` be answered without looking at the digits.
#[derive(Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    BigInt(Rc<BigInt>),
    Float(f64),
    String(Rc<str>),
    Table(Rc<RefCell<VmTable>>),
    Closure(Rc<VmClosure>),
    Native(&'static NativeDef),
    Buffer(Rc<RefCell<VmBuffer>>),
}

pub struct VmTable {
    pub array: Vec<Value>,
    pub fields: FxHashMap<Rc<str>, Value>,
    pub frozen: bool,
    pub metatable: Option<Rc<RefCell<VmTable>>>,
}

impl Default for VmTable {
    fn default() -> Self {
        Self::new()
    }
}

impl VmTable {
    pub fn new() -> Self {
        Self {
            array: Vec::new(),
            fields: new_map(),
            frozen: false,
            metatable: None,
        }
    }

    pub fn with_capacity(array: usize, fields: usize) -> Self {
        let mut map = new_map();
        map.reserve(fields);
        Self {
            array: Vec::with_capacity(array),
            fields: map,
            frozen: false,
            metatable: None,
        }
    }

    pub fn get_str(&self, key: &str) -> Value {
        self.fields.get(key).cloned().unwrap_or(Value::Nil)
    }

    pub fn set_str(&mut self, key: &str, val: Value) {
        if !self.frozen {
            self.fields.insert(Rc::from(key), val);
        }
    }

    /// Looks a metamethod up on this table's metatable, if it has one.
    pub fn metamethod(&self, event: &str) -> Option<Value> {
        let mt = self.metatable.as_ref()?;
        match mt.borrow().fields.get(event) {
            Some(Value::Nil) | None => None,
            Some(h) => Some(h.clone()),
        }
    }
}

/// A captured local. While the local's frame is live the upvalue is *open*:
/// the register at `location` on the current stack holds the value. When the
/// frame returns (or its stack is parked for a coroutine switch) the value
/// moves into `closed`. Keeping the register authoritative while open is what
/// lets ordinary register access skip any upvalue bookkeeping.
pub struct Upvalue {
    /// The stack index while open, `None` once closed.
    pub location: Cell<Option<usize>>,
    pub closed: RefCell<Value>,
}

impl Upvalue {
    pub fn open(index: usize) -> Rc<Upvalue> {
        Rc::new(Upvalue {
            location: Cell::new(Some(index)),
            closed: RefCell::new(Value::Nil),
        })
    }

    pub fn closed(value: Value) -> Rc<Upvalue> {
        Rc::new(Upvalue {
            location: Cell::new(None),
            closed: RefCell::new(value),
        })
    }

    #[inline]
    pub fn is_open(&self) -> bool {
        self.location.get().is_some()
    }

    pub fn close(&self, value: Value) {
        *self.closed.borrow_mut() = value;
        self.location.set(None);
    }
}

pub struct VmClosure {
    pub proto: Rc<Proto>,
    pub upvalues: Vec<Rc<Upvalue>>,
}

impl Value {
    #[inline]
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    /// Builds an integer value, storing it inline when it fits an `i64`.
    #[inline]
    pub fn from_bigint(i: BigInt) -> Value {
        match i.to_i64() {
            Some(small) => Value::Int(small),
            None => Value::BigInt(Rc::new(i)),
        }
    }

    #[inline]
    pub fn from_usize(i: usize) -> Value {
        match i64::try_from(i) {
            Ok(small) => Value::Int(small),
            Err(_) => Value::BigInt(Rc::new(BigInt::from(i))),
        }
    }

    #[inline]
    pub fn from_u64(i: u64) -> Value {
        match i64::try_from(i) {
            Ok(small) => Value::Int(small),
            Err(_) => Value::BigInt(Rc::new(BigInt::from(i))),
        }
    }

    #[inline]
    pub fn str(s: &str) -> Value {
        Value::String(Rc::from(s))
    }

    #[inline]
    pub fn string(s: String) -> Value {
        Value::String(Rc::from(s))
    }

    #[inline]
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Int(_) | Value::BigInt(_) | Value::Float(_))
    }

    /// The integer as a `usize`, if it is a non-negative integer that fits.
    #[inline]
    pub fn as_usize(&self) -> Option<usize> {
        match self {
            Value::Int(i) => usize::try_from(*i).ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Int(_) | Value::BigInt(_) | Value::Float(_) => "number",
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
            Value::Int(_) | Value::BigInt(_) => "bigint",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Closure(_) | Value::Native(..) => "function",
            Value::Buffer(_) => "buffer",
        }
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Value {
        Value::Int(i)
    }
}

impl From<BigInt> for Value {
    fn from(i: BigInt) -> Value {
        Value::from_bigint(i)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::String(Rc::from(s))
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::String(Rc::from(s))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(i) => write!(f, "{}", i),
            Value::BigInt(i) => write!(f, "{}", i),
            Value::Float(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "{}", s),
            Value::Table(_) => write!(f, "table"),
            Value::Closure(c) => write!(
                f,
                "function({})",
                c.proto.name.as_deref().unwrap_or("anonymous")
            ),
            Value::Native(def) => write!(f, "native function({})", def.name),
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
            (Value::BigInt(a), Value::BigInt(b)) => a == b,
            // A normalised BigInt is outside the i64 range, so it can never
            // equal an Int.
            (Value::Int(_), Value::BigInt(_)) | (Value::BigInt(_), Value::Int(_)) => false,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::BigInt(a), Value::Float(b)) => a.to_f64().is_some_and(|v| v == *b),
            (Value::Float(a), Value::BigInt(b)) => b.to_f64().is_some_and(|v| *a == v),
            (Value::String(a), Value::String(b)) => Rc::ptr_eq(a, b) || a == b,
            (Value::Table(a), Value::Table(b)) => Rc::ptr_eq(a, b),
            (Value::Closure(a), Value::Closure(b)) => Rc::ptr_eq(a, b),
            (Value::Native(a), Value::Native(b)) => std::ptr::eq(*a, *b) || a.name == b.name,
            (Value::Buffer(a), Value::Buffer(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}
