use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{FromPrimitive, ToPrimitive, Zero};
use rand::{Rng, RngCore};

use crate::parser::{Expr, Param, Stmt};

const BUNDLED_LIBRARIES: &[(&str, &str)] =
    include!(concat!(env!("OUT_DIR"), "/bundled_libraries.rs"));

pub(crate) type EnvRef = Rc<RefCell<Env>>;
pub(crate) type Native = fn(Vec<Value>) -> Result<Vec<Value>, String>;

/// Integer representation with an `i64` fast path. Most program integers
/// (loop counters, small arithmetic) stay as a plain `i64` with no heap
/// allocation; values that overflow `i64` promote to an `Rc<BigInt>` (`Rc`
/// so cloning a big value, e.g. on every `Value::clone()`, stays O(1)).
#[derive(Clone)]
pub(crate) enum Int {
    Small(i64),
    Big(Rc<BigInt>),
}

impl Int {
    pub(crate) fn from_bigint(value: BigInt) -> Int {
        match value.to_i64() {
            Some(small) => Int::Small(small),
            None => Int::Big(Rc::new(value)),
        }
    }

    pub(crate) fn from_u64(value: u64) -> Int {
        match i64::try_from(value) {
            Ok(small) => Int::Small(small),
            Err(_) => Int::Big(Rc::new(BigInt::from(value))),
        }
    }

    /// The low 64 bits in two's complement, as an unsigned word.
    pub(crate) fn low_u64(&self) -> u64 {
        match self {
            Int::Small(value) => *value as u64,
            Int::Big(value) => ((**value).clone() & BigInt::from(u64::MAX))
                .to_u64()
                .unwrap_or(0),
        }
    }

    pub(crate) fn to_bigint(&self) -> BigInt {
        match self {
            Int::Small(value) => BigInt::from(*value),
            Int::Big(value) => (**value).clone(),
        }
    }

    pub(crate) fn to_f64(&self) -> Option<f64> {
        match self {
            Int::Small(value) => Some(*value as f64),
            Int::Big(value) => value.to_f64(),
        }
    }

    pub(crate) fn to_u32(&self) -> Option<u32> {
        match self {
            Int::Small(value) => u32::try_from(*value).ok(),
            Int::Big(value) => value.to_u32(),
        }
    }

    pub(crate) fn to_i64(&self) -> Option<i64> {
        match self {
            Int::Small(value) => Some(*value),
            Int::Big(value) => value.to_i64(),
        }
    }

    pub(crate) fn to_usize(&self) -> Option<usize> {
        match self {
            Int::Small(value) => usize::try_from(*value).ok(),
            Int::Big(value) => value.to_usize(),
        }
    }

    pub(crate) fn is_zero(&self) -> bool {
        match self {
            Int::Small(value) => *value == 0,
            Int::Big(value) => value.is_zero(),
        }
    }

    pub(crate) fn is_positive(&self) -> bool {
        match self {
            Int::Small(value) => *value > 0,
            Int::Big(value) => value.sign() == Sign::Plus,
        }
    }

    pub(crate) fn is_negative(&self) -> bool {
        match self {
            Int::Small(value) => *value < 0,
            Int::Big(value) => value.sign() == Sign::Minus,
        }
    }

    pub(crate) fn checked_div_floor(&self, other: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, other)
            && !(*a == i64::MIN && *b == -1)
        {
            return Int::Small(a.div_floor(b));
        }
        Int::from_bigint(self.to_bigint().div_floor(&other.to_bigint()))
    }

    pub(crate) fn mod_floor(&self, other: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, other)
            && !(*a == i64::MIN && *b == -1)
        {
            return Int::Small(a.mod_floor(b));
        }
        Int::from_bigint(self.to_bigint().mod_floor(&other.to_bigint()))
    }

    pub(crate) fn shl(&self, bits: usize) -> Int {
        if let Int::Small(value) = self
            && bits < 64
            && let Some(result) = value
                .checked_shl(bits as u32)
                .filter(|result| (*result >> bits) == *value)
        {
            return Int::Small(result);
        }
        Int::from_bigint(self.to_bigint() << bits)
    }

    pub(crate) fn shr(&self, bits: usize) -> Int {
        if let Int::Small(value) = self
            && bits < 64
        {
            return Int::Small(value >> bits.min(63));
        }
        Int::from_bigint(self.to_bigint() >> bits)
    }

    pub(crate) fn pow(&self, exponent: u32) -> Int {
        if let Int::Small(base) = self
            && let Some(result) = base.checked_pow(exponent)
        {
            return Int::Small(result);
        }
        Int::from_bigint(num_traits::Pow::pow(self.to_bigint(), exponent))
    }
}

impl From<i64> for Int {
    fn from(value: i64) -> Int {
        Int::Small(value)
    }
}

impl From<usize> for Int {
    fn from(value: usize) -> Int {
        match i64::try_from(value) {
            Ok(value) => Int::Small(value),
            Err(_) => Int::from_bigint(BigInt::from(value)),
        }
    }
}

impl fmt::Display for Int {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Int::Small(value) => write!(f, "{}", value),
            Int::Big(value) => write!(f, "{}", value),
        }
    }
}

impl PartialEq for Int {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Int::Small(a), Int::Small(b)) => a == b,
            _ => self.to_bigint() == other.to_bigint(),
        }
    }
}

impl Eq for Int {}

impl PartialOrd for Int {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Int {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Int::Small(a), Int::Small(b)) => a.cmp(b),
            _ => self.to_bigint().cmp(&other.to_bigint()),
        }
    }
}

impl std::ops::Add for &Int {
    type Output = Int;
    fn add(self, other: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, other)
            && let Some(result) = a.checked_add(*b)
        {
            return Int::Small(result);
        }
        Int::from_bigint(self.to_bigint() + other.to_bigint())
    }
}

impl std::ops::Sub for &Int {
    type Output = Int;
    fn sub(self, other: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, other)
            && let Some(result) = a.checked_sub(*b)
        {
            return Int::Small(result);
        }
        Int::from_bigint(self.to_bigint() - other.to_bigint())
    }
}

impl std::ops::Mul for &Int {
    type Output = Int;
    fn mul(self, other: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, other)
            && let Some(result) = a.checked_mul(*b)
        {
            return Int::Small(result);
        }
        Int::from_bigint(self.to_bigint() * other.to_bigint())
    }
}

impl std::ops::Neg for Int {
    type Output = Int;
    fn neg(self) -> Int {
        if let Int::Small(value) = self
            && let Some(result) = value.checked_neg()
        {
            return Int::Small(result);
        }
        Int::from_bigint(-self.to_bigint())
    }
}

impl std::ops::BitAnd for &Int {
    type Output = Int;
    fn bitand(self, other: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, other) {
            return Int::Small(a & b);
        }
        Int::from_bigint(self.to_bigint() & other.to_bigint())
    }
}

impl std::ops::BitOr for &Int {
    type Output = Int;
    fn bitor(self, other: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, other) {
            return Int::Small(a | b);
        }
        Int::from_bigint(self.to_bigint() | other.to_bigint())
    }
}

impl std::ops::BitXor for &Int {
    type Output = Int;
    fn bitxor(self, other: &Int) -> Int {
        if let (Int::Small(a), Int::Small(b)) = (self, other) {
            return Int::Small(a ^ b);
        }
        Int::from_bigint(self.to_bigint() ^ other.to_bigint())
    }
}

#[derive(Clone)]
pub(crate) enum Value {
    Nil,
    Bool(bool),
    Number(f64),
    Integer(Int),
    String(String),
    Table(Rc<RefCell<Table>>),
    Function(Rc<Function>),
    Varargs(Vec<Value>),
}

pub(crate) struct Table {
    pub array: Vec<Value>,
    pub fields: HashMap<String, Value>,
    pub const_fields: HashSet<String>,
    pub frozen: bool,
}

pub(crate) enum Function {
    Native {
        name: &'static str,
        call: Native,
    },
    User {
        params: Vec<Param>,
        body: Vec<Stmt>,
        env: EnvRef,
    },
}

pub(crate) struct Env {
    values: HashMap<String, Value>,
    const_names: HashSet<String>,
    parent: Option<EnvRef>,
}

enum Flow {
    Normal,
    Return(Vec<Value>),
    Break,
    Continue,
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(value) => write!(f, "{}", value),
            Value::Number(value) => write!(f, "{}", value),
            Value::Integer(value) => write!(f, "{}", value),
            Value::String(value) => write!(f, "{}", value),
            Value::Table(_) => write!(f, "table"),
            Value::Function(_) => write!(f, "function"),
            Value::Varargs(values) => write!(f, "varargs({})", values.len()),
        }
    }
}

impl Value {
    fn truthy_bool(&self) -> Result<bool, String> {
        match self {
            Value::Bool(value) => Ok(*value),
            _ => Err("condition must be boolean".to_string()),
        }
    }

    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "float",
            Value::Integer(_) => "bigint",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Function(_) => "function",
            Value::Varargs(_) => "varargs",
        }
    }
}

pub fn run_file(path: &str) -> Result<(), String> {
    let program = crate::compiler::compile_file(path)?;
    let mut runtime = Runtime::new();
    runtime.execute(&program).map(|_| ())
}

pub struct Runtime {
    global: EnvRef,
}

impl Runtime {
    pub fn new() -> Self {
        let env = Rc::new(RefCell::new(Env {
            values: HashMap::new(),
            const_names: HashSet::new(),
            parent: None,
        }));
        for (name, function) in [
            ("print", native("print", builtin_print)),
            ("tostring", native("tostring", builtin_tostring)),
            ("type", native("type", builtin_type)),
            ("typeof", native("typeof", builtin_typeof)),
            ("assert", native("assert", builtin_assert)),
            ("error", native("error", builtin_error)),
            ("int", native("int", builtin_int)),
            ("float", native("float", builtin_float)),
            ("__floor", native("__floor", builtin_floor)),
            ("__sqrt", native("__sqrt", builtin_sqrt)),
            ("__ceil", native("__ceil", builtin_ceil)),
            ("__round", native("__round", builtin_round)),
            ("__sin", native("__sin", builtin_sin)),
            ("__cos", native("__cos", builtin_cos)),
            ("__tan", native("__tan", builtin_tan)),
            ("__asin", native("__asin", builtin_asin)),
            ("__acos", native("__acos", builtin_acos)),
            ("__atan", native("__atan", builtin_atan)),
            ("__atan2", native("__atan2", builtin_atan2)),
            ("__sinh", native("__sinh", builtin_sinh)),
            ("__cosh", native("__cosh", builtin_cosh)),
            ("__tanh", native("__tanh", builtin_tanh)),
            ("__log", native("__log", builtin_log)),
            ("__log10", native("__log10", builtin_log10)),
            ("__pow", native("__pow", builtin_pow)),
            ("__fmod", native("__fmod", builtin_fmod)),
            ("__modf", native("__modf", builtin_modf)),
            ("__frexp", native("__frexp", builtin_frexp)),
            ("__ldexp", native("__ldexp", builtin_ldexp)),
            ("__isfinite", native("__isfinite", builtin_isfinite)),
            ("__isinf", native("__isinf", builtin_isinf)),
            ("__noise", native("__noise", builtin_noise)),
            ("__random_int", native("__random_int", builtin_random_int)),
            (
                "__random_bigint",
                native("__random_bigint", builtin_random_bigint),
            ),
            (
                "__table_unpack",
                native("__table_unpack", builtin_table_unpack),
            ),
            (
                "__table_freeze",
                native("__table_freeze", builtin_table_freeze),
            ),
            (
                "__table_isfrozen",
                native("__table_isfrozen", builtin_table_isfrozen),
            ),
            ("try", native("try", builtin_try)),
            ("require", native("require", builtin_require)),
        ] {
            env.borrow_mut().values.insert(name.to_string(), function);
        }
        for (name, call) in crate::string_lib::NATIVES
            .iter()
            .chain(crate::fs_lib::NATIVES)
            .chain(crate::http_lib::NATIVES)
            .chain(crate::io_lib::NATIVES)
        {
            env.borrow_mut()
                .values
                .insert(name.to_string(), native(name, *call));
        }
        Self { global: env }
    }

    fn execute(&mut self, program: &[Stmt]) -> Result<Vec<Value>, String> {
        match self.exec_block(program, self.global.clone())? {
            Flow::Return(values) => Ok(values),
            Flow::Normal => Ok(Vec::new()),
            Flow::Break | Flow::Continue => Err("loop control used outside a loop".to_string()),
        }
    }

    fn exec_block(&self, block: &[Stmt], env: EnvRef) -> Result<Flow, String> {
        for statement in block {
            match self.exec_stmt(statement, env.clone())? {
                Flow::Normal => {}
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_stmt(&self, stmt: &Stmt, env: EnvRef) -> Result<Flow, String> {
        match stmt {
            Stmt::Local {
                name,
                is_const,
                initializer,
                ..
            } => {
                let value = initializer
                    .as_ref()
                    .map(|expr| self.eval(expr, env.clone()))
                    .transpose()?
                    .unwrap_or(Value::Nil);
                let value = first_value(value);
                let mut env = env.borrow_mut();
                env.values.insert(name.clone(), value);
                if *is_const {
                    env.const_names.insert(name.clone());
                }
            }
            Stmt::LocalMany {
                names,
                is_const,
                initializers,
            } => {
                let mut values = Vec::new();
                for expr in initializers {
                    match self.eval(expr, env.clone())? {
                        Value::Varargs(varargs) => values.extend(varargs),
                        value => values.push(value),
                    }
                }
                for (index, name) in names.iter().enumerate() {
                    env.borrow_mut().values.insert(
                        name.clone(),
                        values.get(index).cloned().unwrap_or(Value::Nil),
                    );
                    if *is_const {
                        env.borrow_mut().const_names.insert(name.clone());
                    }
                }
            }
            Stmt::Assign {
                target,
                value,
                is_const,
            } => {
                let value = first_value(self.eval(value, env.clone())?);
                self.assign(target, value, env.clone())?;
                if *is_const {
                    self.protect_member(target, env)?;
                }
            }
            Stmt::AssignMany { targets, values } => {
                let mut evaluated = Vec::new();
                for expr in values {
                    match self.eval(expr, env.clone())? {
                        Value::Varargs(varargs) => evaluated.extend(varargs),
                        value => evaluated.push(value),
                    }
                }
                for (index, target) in targets.iter().enumerate() {
                    let value = evaluated.get(index).cloned().unwrap_or(Value::Nil);
                    self.assign(target, value, env.clone())?;
                }
            }
            Stmt::Increment { target, amount } => {
                let current = self.eval(target, env.clone())?;
                let value =
                    self.numeric(current, "+", Value::Integer(Int::from(*amount as i64)))?;
                self.assign(target, value, env)?;
            }
            Stmt::Function {
                name: Some(name),
                is_const,
                params,
                body,
                ..
            } => {
                let value = Value::Function(Rc::new(Function::User {
                    params: params.clone(),
                    body: body.clone(),
                    env: env.clone(),
                }));
                if name.contains('.') {
                    let mut target = Expr::Variable(name.split('.').next().unwrap().to_string());
                    for field in name.split('.').skip(1) {
                        target = Expr::Member {
                            object: Box::new(target),
                            field: field.to_string(),
                        };
                    }
                    self.assign(&target, value, env.clone())?;
                    if *is_const {
                        self.protect_member(&target, env)?;
                    }
                } else {
                    let mut env = env.borrow_mut();
                    env.values.insert(name.clone(), value);
                    if *is_const {
                        env.const_names.insert(name.clone());
                    }
                }
            }
            Stmt::Function { name: None, .. } => {
                return Err("anonymous function is only valid as an expression".to_string());
            }
            Stmt::Expr(expr) => {
                self.eval(expr, env)?;
            }
            Stmt::Return(exprs) => {
                let mut values = Vec::new();
                for expr in exprs {
                    match self.eval(expr, env.clone())? {
                        Value::Varargs(varargs) => values.extend(varargs),
                        value => values.push(value),
                    }
                }
                if values.is_empty() {
                    values.push(Value::Nil);
                }
                return Ok(Flow::Return(values));
            }
            Stmt::If {
                condition,
                then_branch,
                else_if_branches,
                else_branch,
            } => {
                if self.eval(condition, env.clone())?.truthy_bool()? {
                    return self.exec_block(then_branch, child(&env));
                }
                for (condition, branch) in else_if_branches {
                    if self.eval(condition, env.clone())?.truthy_bool()? {
                        return self.exec_block(branch, child(&env));
                    }
                }
                if let Some(branch) = else_branch {
                    return self.exec_block(branch, child(&env));
                }
            }
            Stmt::While { condition, body } => {
                while self.eval(condition, env.clone())?.truthy_bool()? {
                    match self.exec_block(body, child(&env))? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow => return Ok(flow),
                    }
                }
            }
            Stmt::Repeat { body, condition } => loop {
                match self.exec_block(body, child(&env))? {
                    Flow::Normal | Flow::Continue => {}
                    Flow::Break => break,
                    flow => return Ok(flow),
                }
                if self.eval(condition, env.clone())?.truthy_bool()? {
                    break;
                }
            },
            Stmt::For { vars, source, body } => {
                let table = first_value(self.eval(source, env.clone())?);
                if let Value::Function(_) = table {
                    // Iterator function: call it until its first result is nil.
                    loop {
                        let values = self.call(table.clone(), Vec::new())?;
                        if matches!(values.first(), None | Some(Value::Nil)) {
                            break;
                        }
                        let loop_env = child(&env);
                        for (index, name) in vars.iter().enumerate() {
                            loop_env.borrow_mut().values.insert(
                                name.clone(),
                                values.get(index).cloned().unwrap_or(Value::Nil),
                            );
                        }
                        match self.exec_block(body, loop_env)? {
                            Flow::Normal | Flow::Continue => {}
                            Flow::Break => break,
                            flow => return Ok(flow),
                        }
                    }
                    return Ok(Flow::Normal);
                }
                let values: Vec<Vec<Value>> = match table {
                    Value::Table(table) => {
                        let table = table.borrow();
                        if vars.len() > 1 && !table.array.is_empty() {
                            table
                                .array
                                .iter()
                                .enumerate()
                                .map(|(i, v)| vec![Value::Integer(Int::from(i + 1)), v.clone()])
                                .collect()
                        } else if vars.len() > 1 {
                            table
                                .fields
                                .iter()
                                .map(|(key, value)| vec![Value::String(key.clone()), value.clone()])
                                .collect()
                        } else {
                            table.array.iter().map(|v| vec![v.clone()]).collect()
                        }
                    }
                    _ => return Err("generic for expects a table or iterator function".to_string()),
                };
                for values in values {
                    let loop_env = child(&env);
                    for (index, name) in vars.iter().enumerate() {
                        loop_env.borrow_mut().values.insert(
                            name.clone(),
                            values.get(index).cloned().unwrap_or(Value::Nil),
                        );
                    }
                    match self.exec_block(body, loop_env)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow => return Ok(flow),
                    }
                }
            }
            Stmt::NumericFor {
                var,
                start,
                end,
                step,
                body,
            } => {
                let mut current = number(self.eval(start, env.clone())?)?;
                let limit = number(self.eval(end, env.clone())?)?;
                let increment = match step {
                    Some(step) => number(self.eval(step, env.clone())?)?,
                    None => 1.0,
                };
                if increment == 0.0 {
                    return Err("numeric for step cannot be zero".to_string());
                }

                while (increment > 0.0 && current <= limit) || (increment < 0.0 && current >= limit)
                {
                    let loop_env = child(&env);
                    let loop_value = if current.fract() == 0.0 {
                        Value::Integer(Int::from_bigint(BigInt::from_f64(current).unwrap()))
                    } else {
                        Value::Number(current)
                    };
                    loop_env.borrow_mut().values.insert(var.clone(), loop_value);
                    match self.exec_block(body, loop_env)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow => return Ok(flow),
                    }
                    current += increment;
                }
            }
            Stmt::Break => return Ok(Flow::Break),
            Stmt::Continue => return Ok(Flow::Continue),
        }
        Ok(Flow::Normal)
    }

    fn eval(&self, expr: &Expr, env: EnvRef) -> Result<Value, String> {
        match expr {
            Expr::Literal(value) => parse_literal(value),
            Expr::Str(value) => Ok(Value::String(value.clone())),
            Expr::Interp(value) => self.interpolate(value, env),
            Expr::Variable(name) => {
                lookup(&env, name).ok_or_else(|| format!("undefined name `{}`", name))
            }
            Expr::Vararg => {
                let values = lookup(&env, "__varargs")
                    .ok_or_else(|| "vararg expression outside a variadic function".to_string())?;
                match values {
                    Value::Table(values) => Ok(Value::Varargs(values.borrow().array.clone())),
                    _ => Ok(values),
                }
            }
            Expr::Member { object, field } => {
                self.index(&self.eval(object, env)?, &Value::String(field.clone()))
            }
            Expr::Index { object, index } => {
                self.index(&self.eval(object, env.clone())?, &self.eval(index, env)?)
            }
            Expr::Table(entries) => {
                let mut table = Table {
                    array: Vec::new(),
                    fields: HashMap::new(),
                    const_fields: HashSet::new(),
                    frozen: false,
                };
                for entry in entries {
                    if matches!(entry.value, Expr::Vararg) {
                        if let Some(Value::Table(values)) = lookup(&env, "__varargs") {
                            table.array.extend(values.borrow().array.iter().cloned());
                        }
                        continue;
                    }
                    let value = self.eval(&entry.value, env.clone())?;
                    match value {
                        Value::Varargs(values) => {
                            if let Some(key) = &entry.key {
                                table.fields.insert(key.clone(), Value::Varargs(values));
                            } else {
                                table.array.extend(values);
                            }
                        }
                        value => {
                            if let Some(key) = &entry.key {
                                table.fields.insert(key.clone(), value);
                            } else {
                                table.array.push(value);
                            }
                        }
                    }
                }
                Ok(Value::Table(Rc::new(RefCell::new(table))))
            }
            Expr::Unary { op, expr } => {
                let value = first_value(self.eval(expr, env)?);
                match op.as_str() {
                    "not" => Ok(Value::Bool(!value.truthy_bool()?)),
                    "-" => self.number_unary(value, true),
                    "#" => self.length(value),
                    _ => Err(format!("unsupported unary operator {}", op)),
                }
            }
            Expr::Binary { left, op, right } => self.binary(left, op, right, env),
            Expr::Call { callee, args } => {
                let function = self.eval(callee, env.clone())?;
                let values = self.eval_args(args, env)?;
                self.call(function, values).map(collapse_values)
            }
            Expr::MethodCall {
                object,
                method,
                args,
            } => {
                // `object:method(args)` looks `method` up on the object and
                // passes the object itself as the first argument.
                let receiver = first_value(self.eval(object, env.clone())?);
                if !matches!(receiver, Value::Table(_)) {
                    return Err(format!(
                        "cannot call method `{}` on a {}",
                        method,
                        receiver.type_name()
                    ));
                }
                let function = self.index(&receiver, &Value::String(method.clone()))?;
                if matches!(function, Value::Nil) {
                    return Err(format!("method `{}` is not defined", method));
                }
                let mut values = vec![receiver];
                values.extend(self.eval_args(args, env)?);
                self.call(function, values).map(collapse_values)
            }
            Expr::Function { params, body } => Ok(Value::Function(Rc::new(Function::User {
                params: params.clone(),
                body: body.clone(),
                env,
            }))),
        }
    }

    fn eval_args(&self, args: &[Expr], env: EnvRef) -> Result<Vec<Value>, String> {
        let mut values = Vec::new();
        for arg in args {
            if matches!(arg, Expr::Vararg) {
                if let Some(Value::Table(varargs)) = lookup(&env, "__varargs") {
                    values.extend(varargs.borrow().array.iter().cloned());
                }
            } else {
                let value = self.eval(arg, env.clone())?;
                match value {
                    Value::Varargs(varargs) => values.extend(varargs),
                    value => values.push(value),
                }
            }
        }
        Ok(values)
    }

    fn call(&self, function: Value, args: Vec<Value>) -> Result<Vec<Value>, String> {
        match function {
            Value::Function(function) => match &*function {
                Function::Native { name: "try", .. } => self.call_try(args),
                Function::Native {
                    name: "require", ..
                } => self.call_require(args),
                Function::Native { call, .. } => call(args),
                Function::User { params, body, env } => {
                    let call_env = child(env);
                    let mut arg_index = 0;
                    for param in params {
                        if param.variadic {
                            let values = Table {
                                array: args[arg_index..].to_vec(),
                                fields: HashMap::new(),
                                const_fields: HashSet::new(),
                                frozen: false,
                            };
                            call_env.borrow_mut().values.insert(
                                "__varargs".to_string(),
                                Value::Table(Rc::new(RefCell::new(values))),
                            );
                            arg_index = args.len();
                        } else {
                            call_env.borrow_mut().values.insert(
                                param.name.clone(),
                                args.get(arg_index).cloned().unwrap_or(Value::Nil),
                            );
                            arg_index += 1;
                        }
                    }
                    match self.exec_block(body, call_env)? {
                        Flow::Return(values) => Ok(values),
                        _ => Ok(vec![Value::Nil]),
                    }
                }
            },
            _ => Err("value is not callable".to_string()),
        }
    }

    fn call_try(&self, args: Vec<Value>) -> Result<Vec<Value>, String> {
        let Some(function) = args.first().cloned() else {
            return Err("try expects a function".to_string());
        };
        match self.call(function, args[1..].to_vec()) {
            Ok(mut values) => {
                values.insert(0, Value::Bool(true));
                Ok(values)
            }
            Err(error) => Ok(vec![Value::Bool(false), Value::String(error)]),
        }
    }

    fn call_require(&self, args: Vec<Value>) -> Result<Vec<Value>, String> {
        let Some(Value::String(package)) = args.first() else {
            return Err("require expects a string path".to_string());
        };
        let Some((_, source)) = BUNDLED_LIBRARIES.iter().find(|(name, _)| *name == package) else {
            return Err(format!(
                "package `{}` is not bundled; add it to the project manually",
                package
            ));
        };
        let program = crate::compiler::compile_source(source)?;
        let module_env = child(&self.global);
        match self.exec_block(&program, module_env)? {
            Flow::Return(values) => Ok(values),
            Flow::Normal => Ok(vec![Value::Nil]),
            Flow::Break | Flow::Continue => Err("loop control used outside a loop".to_string()),
        }
    }

    fn interpolate(
        &self,
        parts: &[crate::parser::InterpPart],
        env: EnvRef,
    ) -> Result<Value, String> {
        let mut output = String::new();
        for part in parts {
            match part {
                crate::parser::InterpPart::Literal(text) => output.push_str(text),
                crate::parser::InterpPart::Expr(expr) => {
                    output.push_str(&self.eval(expr, env.clone())?.to_string())
                }
            }
        }
        Ok(Value::String(output))
    }

    fn binary(&self, left: &Expr, op: &str, right: &Expr, env: EnvRef) -> Result<Value, String> {
        let left = first_value(self.eval(left, env.clone())?);
        if op == "and" || op == "or" {
            let a = left.truthy_bool()?;
            if (op == "and" && !a) || (op == "or" && a) {
                return Ok(Value::Bool(a));
            }
            return Ok(Value::Bool(
                first_value(self.eval(right, env)?).truthy_bool()?,
            ));
        }
        let right = first_value(self.eval(right, env)?);
        match op {
            "??" => {
                if matches!(left, Value::Nil) {
                    Ok(right)
                } else {
                    Ok(left)
                }
            }
            ".." => Ok(Value::String(format!(
                "{}{}",
                require_string(left)?,
                require_string(right)?
            ))),
            "+" | "-" | "*" | "/" | "//" | "%" | "^" => self.numeric(left, op, right),
            "==" => Ok(Value::Bool(equal(&left, &right))),
            "!=" => Ok(Value::Bool(!equal(&left, &right))),
            "<" | "<=" | ">" | ">=" => compare(left, op, right),
            "&" | "|" | "~" | "<<" | ">>" | "<<<" | ">>>" => bitwise(left, op, right),
            _ => Err(format!("unsupported operator {}", op)),
        }
    }

    fn numeric(&self, left: Value, op: &str, right: Value) -> Result<Value, String> {
        if let (Value::Integer(a), Value::Integer(b)) = (&left, &right) {
            if (op == "//" || op == "%") && b.is_zero() {
                return Err("division by zero".to_string());
            }
            return match op {
                "+" => Ok(Value::Integer(a + b)),
                "-" => Ok(Value::Integer(a - b)),
                "*" => Ok(Value::Integer(a * b)),
                "/" => Ok(Value::Number(number(left)? / number(right)?)),
                "//" => Ok(Value::Integer(a.checked_div_floor(b))),
                "%" => Ok(Value::Integer(a.mod_floor(b))),
                "^" if !b.is_negative() => {
                    let exponent = b
                        .to_u32()
                        .ok_or_else(|| "integer exponent is too large".to_string())?;
                    Ok(Value::Integer(a.pow(exponent)))
                }
                _ => Ok(Value::Number(number(left)?.powf(number(right)?))),
            };
        }
        let a = number(left)?;
        let b = number(right)?;
        if (op == "//" || op == "%") && b == 0.0 {
            return Err("division by zero".to_string());
        }
        if op == "/" {
            return Ok(Value::Number(a / b));
        }
        let result = match op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "//" => (a / b).floor(),
            "%" => a - (a / b).floor() * b,
            "^" => a.powf(b),
            _ => unreachable!(),
        };
        if result.fract() == 0.0 && result.is_finite() {
            Ok(Value::Integer(Int::from_bigint(
                BigInt::from_f64(result)
                    .ok_or_else(|| "integer result is out of range".to_string())?,
            )))
        } else {
            Ok(Value::Number(result))
        }
    }
    fn number_unary(&self, value: Value, negate: bool) -> Result<Value, String> {
        match value {
            Value::Integer(value) if negate => Ok(Value::Integer(-value)),
            Value::Integer(value) => Ok(Value::Integer(value)),
            Value::Number(value) if negate => Ok(Value::Number(-value)),
            Value::Number(value) => Ok(Value::Number(value)),
            _ => Err("expected a number".to_string()),
        }
    }
    fn length(&self, value: Value) -> Result<Value, String> {
        match value {
            Value::String(value) => Ok(Value::Integer(Int::from(value.len()))),
            Value::Table(value) => Ok(Value::Integer(Int::from(value.borrow().array.len()))),
            _ => Err("length expects a string or table".to_string()),
        }
    }
    fn index(&self, object: &Value, index: &Value) -> Result<Value, String> {
        match object {
            Value::Table(table) => {
                let table = table.borrow();
                match index {
                    Value::String(key) => Ok(table.fields.get(key).cloned().unwrap_or(Value::Nil)),
                    Value::Integer(index) if index.is_positive() => Ok(table
                        .array
                        .get(
                            index
                                .to_usize()
                                .ok_or_else(|| "table index is too large".to_string())?
                                - 1,
                        )
                        .cloned()
                        .unwrap_or(Value::Nil)),
                    _ => Err("table index must be a string or positive integer".to_string()),
                }
            }
            Value::String(_) => {
                Err("string members require a user-provided string package".to_string())
            }
            _ => Err("value is not indexable".to_string()),
        }
    }
    fn assign(&self, target: &Expr, value: Value, env: EnvRef) -> Result<(), String> {
        match target {
            Expr::Variable(name) => assign_env(&env, name, value),
            Expr::Member { object, field } => self.assign_index(
                &self.eval(object, env)?,
                Value::String(field.clone()),
                value,
            ),
            Expr::Index { object, index } => self.assign_index(
                &self.eval(object, env.clone())?,
                self.eval(index, env)?,
                value,
            ),
            _ => Err("invalid assignment target".to_string()),
        }
    }
    fn assign_index(&self, object: &Value, index: Value, value: Value) -> Result<(), String> {
        match object {
            Value::Table(table) => {
                let mut table = table.borrow_mut();
                if table.frozen {
                    return Err("assignment to frozen table".to_string());
                }
                match index {
                    Value::String(key) => {
                        if table.const_fields.contains(&key) {
                            return Err(format!("assignment to const field `{}`", key));
                        }
                        table.fields.insert(key, value);
                        Ok(())
                    }
                    Value::Integer(index) if index.is_positive() => {
                        let index = index
                            .to_usize()
                            .ok_or_else(|| "table index is too large".to_string())?;
                        while table.array.len() < index {
                            table.array.push(Value::Nil);
                        }
                        table.array[index - 1] = value;
                        Ok(())
                    }
                    _ => Err("invalid table index".to_string()),
                }
            }
            _ => Err("assignment target is not a table".to_string()),
        }
    }

    fn protect_member(&self, target: &Expr, env: EnvRef) -> Result<(), String> {
        let Expr::Member { object, field } = target else {
            return Err("invalid const function target".to_string());
        };
        let value = self.eval(object, env)?;
        let Value::Table(table) = value else {
            return Err("const function target is not a table".to_string());
        };
        table.borrow_mut().const_fields.insert(field.clone());
        Ok(())
    }
}

/// The result list of a call becomes a single value, or varargs when the
/// callee returned several.
fn collapse_values(values: Vec<Value>) -> Value {
    match values.as_slice() {
        [] => Value::Nil,
        [value] => value.clone(),
        _ => Value::Varargs(values),
    }
}
/// A multi-value result used where a single value is expected keeps only its
/// first value (or nil when it is empty).
fn first_value(value: Value) -> Value {
    match value {
        Value::Varargs(values) => values.into_iter().next().unwrap_or(Value::Nil),
        value => value,
    }
}
fn native(name: &'static str, call: Native) -> Value {
    Value::Function(Rc::new(Function::Native { name, call }))
}
pub(crate) fn new_table(array: Vec<Value>) -> Value {
    Value::Table(Rc::new(RefCell::new(Table {
        array,
        fields: HashMap::new(),
        const_fields: HashSet::new(),
        frozen: false,
    })))
}
fn child(parent: &EnvRef) -> EnvRef {
    Rc::new(RefCell::new(Env {
        values: HashMap::new(),
        const_names: HashSet::new(),
        parent: Some(parent.clone()),
    }))
}
fn lookup(env: &EnvRef, name: &str) -> Option<Value> {
    let borrowed = env.borrow();
    if let Some(value) = borrowed.values.get(name) {
        return Some(value.clone());
    }
    let parent = borrowed.parent.clone();
    drop(borrowed);
    parent.and_then(|parent| lookup(&parent, name))
}
fn assign_env(env: &EnvRef, name: &str, value: Value) -> Result<(), String> {
    let mut borrowed = env.borrow_mut();
    if borrowed.values.contains_key(name) {
        if borrowed.const_names.contains(name) {
            return Err(format!("assignment to const name `{}`", name));
        }
        borrowed.values.insert(name.to_string(), value);
        Ok(())
    } else {
        let parent = borrowed.parent.clone();
        drop(borrowed);
        if let Some(parent) = parent {
            assign_env(&parent, name, value)
        } else {
            Err(format!("assignment to undefined name `{}`", name))
        }
    }
}
fn parse_literal(value: &str) -> Result<Value, String> {
    match value {
        "nil" => Ok(Value::Nil),
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        _ => {
            let normalized = value.replace('_', "");
            let integer = if let Some(value) = normalized.strip_prefix("0x") {
                BigInt::parse_bytes(value.as_bytes(), 16)
            } else if let Some(value) = normalized.strip_prefix("0X") {
                BigInt::parse_bytes(value.as_bytes(), 16)
            } else if let Some(value) = normalized.strip_prefix("0b") {
                BigInt::parse_bytes(value.as_bytes(), 2)
            } else if let Some(value) = normalized.strip_prefix("0B") {
                BigInt::parse_bytes(value.as_bytes(), 2)
            } else {
                BigInt::parse_bytes(normalized.as_bytes(), 10)
            };
            if let Some(value) = integer {
                Ok(Value::Integer(Int::from_bigint(value)))
            } else if let Ok(value) = normalized.parse::<f64>() {
                Ok(Value::Number(value))
            } else {
                Ok(Value::String(value.to_string()))
            }
        }
    }
}
pub(crate) fn number(value: Value) -> Result<f64, String> {
    match value {
        Value::Integer(value) => value
            .to_f64()
            .ok_or_else(|| "integer is too large for floating-point conversion".to_string()),
        Value::Number(value) => Ok(value),
        _ => Err("expected a number".to_string()),
    }
}
pub(crate) fn require_string(value: Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value),
        _ => Err("expected a string".to_string()),
    }
}
fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Integer(a), Value::Number(b)) | (Value::Number(b), Value::Integer(a)) => {
            a.to_f64().is_some_and(|a| a == *b)
        }
        (Value::Number(a), Value::Number(b)) => a == b,
        // Tables and functions are reference types: equal only to themselves.
        (Value::Table(a), Value::Table(b)) => Rc::ptr_eq(a, b),
        (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}
fn compare(left: Value, op: &str, right: Value) -> Result<Value, String> {
    let result = match (&left, &right) {
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
        _ => number(left)?
            .partial_cmp(&number(right)?)
            .ok_or_else(|| "values are not comparable".to_string())?,
    };
    Ok(Value::Bool(match op {
        "<" => result.is_lt(),
        "<=" => result.is_le(),
        ">" => result.is_gt(),
        ">=" => result.is_ge(),
        _ => false,
    }))
}
fn bitwise(left: Value, op: &str, right: Value) -> Result<Value, String> {
    let a = match left {
        Value::Integer(value) => value,
        value => Int::from_bigint(
            BigInt::from_f64(number(value)?.trunc())
                .ok_or_else(|| "expected an integer".to_string())?,
        ),
    };
    let b = match right {
        Value::Integer(value) => value,
        value => Int::from_bigint(
            BigInt::from_f64(number(value)?.trunc())
                .ok_or_else(|| "expected an integer".to_string())?,
        ),
    };
    Ok(Value::Integer(match op {
        "&" => &a & &b,
        "|" => &a | &b,
        "~" => &a ^ &b,
        "<<" => a.shl(
            b.to_usize()
                .ok_or_else(|| "shift is too large".to_string())?,
        ),
        ">>" => a.shr(
            b.to_usize()
                .ok_or_else(|| "shift is too large".to_string())?,
        ),
        "<<<" | ">>>" => {
            // Logical shifts act on the low 64 bits as an unsigned word, so
            // the result is always in 0..2^64 and shifting by 64+ yields 0.
            let word = a.low_u64();
            let bits = b.to_usize().unwrap_or(usize::MAX);
            Int::from_u64(match (op, bits) {
                (_, 64..) => 0,
                ("<<<", _) => word << bits,
                (_, _) => word >> bits,
            })
        }
        _ => return Err("unsupported bitwise operator".to_string()),
    }))
}
fn builtin_print(args: Vec<Value>) -> Result<Vec<Value>, String> {
    println!(
        "{}",
        args.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\t")
    );
    Ok(vec![Value::Nil])
}
fn builtin_tostring(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::String(
        args.first().cloned().unwrap_or(Value::Nil).to_string(),
    )])
}
fn builtin_type(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::String(
        match args.first().unwrap_or(&Value::Nil) {
            Value::Integer(_) | Value::Number(_) => "number",
            value => value.type_name(),
        }
        .to_string(),
    )])
}
fn builtin_typeof(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::String(
        args.first().unwrap_or(&Value::Nil).type_name().to_string(),
    )])
}
fn builtin_assert(args: Vec<Value>) -> Result<Vec<Value>, String> {
    if !args.first().unwrap_or(&Value::Nil).truthy_bool()? {
        return Err(args
            .get(1)
            .map(ToString::to_string)
            .unwrap_or_else(|| "assertion failed".to_string()));
    }
    Ok(args)
}
fn builtin_error(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Err(args
        .first()
        .map(ToString::to_string)
        .unwrap_or_else(|| "error".to_string()))
}
fn builtin_int(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = args.first().cloned().unwrap_or(Value::Nil);
    Ok(vec![match value {
        Value::Integer(value) => Value::Integer(value),
        value => Value::Integer(Int::from_bigint(
            BigInt::from_f64(number(value)?.trunc())
                .ok_or_else(|| "value cannot be converted to an integer".to_string())?,
        )),
    }])
}
fn builtin_float(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Number(number(
        args.first().cloned().unwrap_or(Value::Nil),
    )?)])
}
fn builtin_floor(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = args.first().cloned().unwrap_or(Value::Nil);
    Ok(vec![match value {
        Value::Integer(value) => Value::Integer(value),
        value => Value::Integer(Int::from_bigint(
            BigInt::from_f64(number(value)?.floor())
                .ok_or_else(|| "value cannot be converted to an integer".to_string())?,
        )),
    }])
}
fn builtin_sqrt(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = number(args.first().cloned().unwrap_or(Value::Nil))?;
    if !value.is_finite() {
        return Err("sqrt expects a finite number".to_string());
    }
    if value < 0.0 {
        return Err("sqrt expects a non-negative number".to_string());
    }
    Ok(vec![Value::Number(value.sqrt())])
}
fn float_argument(args: &[Value], index: usize, name: &str) -> Result<f64, String> {
    number(args.get(index).cloned().unwrap_or(Value::Nil))
        .map_err(|_| format!("{} must be a number", name))
}
fn float_to_integer(value: f64) -> Result<Value, String> {
    Ok(Value::Integer(Int::from_bigint(
        BigInt::from_f64(value)
            .ok_or_else(|| "value cannot be converted to an integer".to_string())?,
    )))
}
fn builtin_ceil(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![match args.first().cloned().unwrap_or(Value::Nil) {
        Value::Integer(value) => Value::Integer(value),
        value => float_to_integer(number(value)?.ceil())?,
    }])
}
fn builtin_round(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![match args.first().cloned().unwrap_or(Value::Nil) {
        Value::Integer(value) => Value::Integer(value),
        value => float_to_integer(number(value)?.round())?,
    }])
}
fn unary_float(args: Vec<Value>, name: &str, f: fn(f64) -> f64) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Number(f(float_argument(&args, 0, name)?))])
}
fn builtin_sin(args: Vec<Value>) -> Result<Vec<Value>, String> {
    unary_float(args, "sin argument", f64::sin)
}
fn builtin_cos(args: Vec<Value>) -> Result<Vec<Value>, String> {
    unary_float(args, "cos argument", f64::cos)
}
fn builtin_tan(args: Vec<Value>) -> Result<Vec<Value>, String> {
    unary_float(args, "tan argument", f64::tan)
}
fn builtin_asin(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = float_argument(&args, 0, "asin argument")?;
    if !(-1.0..=1.0).contains(&value) {
        return Err("asin expects a number between -1 and 1".to_string());
    }
    Ok(vec![Value::Number(value.asin())])
}
fn builtin_acos(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = float_argument(&args, 0, "acos argument")?;
    if !(-1.0..=1.0).contains(&value) {
        return Err("acos expects a number between -1 and 1".to_string());
    }
    Ok(vec![Value::Number(value.acos())])
}
fn builtin_atan(args: Vec<Value>) -> Result<Vec<Value>, String> {
    unary_float(args, "atan argument", f64::atan)
}
fn builtin_atan2(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let y = float_argument(&args, 0, "atan2 y")?;
    let x = float_argument(&args, 1, "atan2 x")?;
    Ok(vec![Value::Number(y.atan2(x))])
}
fn builtin_sinh(args: Vec<Value>) -> Result<Vec<Value>, String> {
    unary_float(args, "sinh argument", f64::sinh)
}
fn builtin_cosh(args: Vec<Value>) -> Result<Vec<Value>, String> {
    unary_float(args, "cosh argument", f64::cosh)
}
fn builtin_tanh(args: Vec<Value>) -> Result<Vec<Value>, String> {
    unary_float(args, "tanh argument", f64::tanh)
}
fn builtin_log(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = float_argument(&args, 0, "log argument")?;
    if value < 0.0 {
        return Err("log expects a non-negative number".to_string());
    }
    let result = match args.get(1) {
        None | Some(Value::Nil) => value.ln(),
        Some(_) => {
            let base = float_argument(&args, 1, "log base")?;
            if base <= 0.0 || base == 1.0 {
                return Err("log base must be positive and not equal to 1".to_string());
            }
            value.log(base)
        }
    };
    Ok(vec![Value::Number(result)])
}
fn builtin_log10(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = float_argument(&args, 0, "log10 argument")?;
    if value < 0.0 {
        return Err("log10 expects a non-negative number".to_string());
    }
    Ok(vec![Value::Number(value.log10())])
}
fn builtin_pow(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let base = float_argument(&args, 0, "pow base")?;
    let exponent = float_argument(&args, 1, "pow exponent")?;
    Ok(vec![Value::Number(base.powf(exponent))])
}
fn builtin_fmod(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let a = float_argument(&args, 0, "fmod a")?;
    let b = float_argument(&args, 1, "fmod b")?;
    if b == 0.0 {
        return Err("fmod divisor must not be zero".to_string());
    }
    Ok(vec![Value::Number(a % b)])
}
fn builtin_modf(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = float_argument(&args, 0, "modf argument")?;
    if value.is_infinite() {
        return Ok(vec![Value::Number(value), Value::Number(0.0)]);
    }
    Ok(vec![
        Value::Number(value.trunc()),
        Value::Number(value.fract()),
    ])
}
fn builtin_frexp(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = float_argument(&args, 0, "frexp argument")?;
    if value == 0.0 || !value.is_finite() {
        return Ok(vec![Value::Number(value), Value::Integer(Int::Small(0))]);
    }
    // Decompose value = mantissa * 2^exponent with 0.5 <= |mantissa| < 1.
    let exponent_mask = 0x7ffu64 << 52;
    let (bits, bias) = if value.to_bits() & exponent_mask == 0 {
        // Subnormal: scale up first so the exponent field is populated.
        ((value * 2f64.powi(64)).to_bits(), 1022 + 64)
    } else {
        (value.to_bits(), 1022)
    };
    let raw_exponent = ((bits & exponent_mask) >> 52) as i64;
    let mantissa = f64::from_bits((bits & !exponent_mask) | (1022u64 << 52));
    Ok(vec![
        Value::Number(mantissa),
        Value::Integer(Int::from(raw_exponent - bias)),
    ])
}
fn builtin_ldexp(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let mantissa = float_argument(&args, 0, "ldexp mantissa")?;
    let exponent = float_argument(&args, 1, "ldexp exponent")?;
    if !exponent.is_finite() || exponent.fract() != 0.0 {
        return Err("ldexp exponent must be an integer".to_string());
    }
    let exponent = exponent.clamp(i32::MIN as f64, i32::MAX as f64) as i32;
    Ok(vec![Value::Number(mantissa * 2f64.powi(exponent))])
}
fn builtin_isfinite(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Bool(
        match args.first().cloned().unwrap_or(Value::Nil) {
            Value::Integer(_) => true,
            value => number(value)?.is_finite(),
        },
    )])
}
fn builtin_isinf(args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Bool(
        match args.first().cloned().unwrap_or(Value::Nil) {
            Value::Integer(_) => false,
            value => number(value)?.is_infinite(),
        },
    )])
}

// Improved Perlin noise (Ken Perlin, 2002). Returns values in roughly [-1, 1];
// integer lattice points always yield 0.
const PERLIN_PERMUTATION: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];
fn perlin_hash(index: i64) -> usize {
    PERLIN_PERMUTATION[index.rem_euclid(256) as usize] as usize
}
fn perlin_fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
fn perlin_lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}
fn perlin_grad(hash: usize, x: f64, y: f64, z: f64) -> f64 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}
fn perlin_noise(x: f64, y: f64, z: f64) -> f64 {
    let (xi, yi, zi) = (x.floor() as i64, y.floor() as i64, z.floor() as i64);
    let (x, y, z) = (x - x.floor(), y - y.floor(), z - z.floor());
    let (u, v, w) = (perlin_fade(x), perlin_fade(y), perlin_fade(z));
    let a = perlin_hash(xi) as i64 + yi;
    let aa = perlin_hash(a) as i64 + zi;
    let ab = perlin_hash(a + 1) as i64 + zi;
    let b = perlin_hash(xi + 1) as i64 + yi;
    let ba = perlin_hash(b) as i64 + zi;
    let bb = perlin_hash(b + 1) as i64 + zi;
    perlin_lerp(
        w,
        perlin_lerp(
            v,
            perlin_lerp(
                u,
                perlin_grad(perlin_hash(aa), x, y, z),
                perlin_grad(perlin_hash(ba), x - 1.0, y, z),
            ),
            perlin_lerp(
                u,
                perlin_grad(perlin_hash(ab), x, y - 1.0, z),
                perlin_grad(perlin_hash(bb), x - 1.0, y - 1.0, z),
            ),
        ),
        perlin_lerp(
            v,
            perlin_lerp(
                u,
                perlin_grad(perlin_hash(aa + 1), x, y, z - 1.0),
                perlin_grad(perlin_hash(ba + 1), x - 1.0, y, z - 1.0),
            ),
            perlin_lerp(
                u,
                perlin_grad(perlin_hash(ab + 1), x, y - 1.0, z - 1.0),
                perlin_grad(perlin_hash(bb + 1), x - 1.0, y - 1.0, z - 1.0),
            ),
        ),
    )
}
fn builtin_noise(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let coordinate = |index: usize, name: &str| -> Result<f64, String> {
        match args.get(index) {
            None | Some(Value::Nil) => Ok(0.0),
            Some(_) => {
                let value = float_argument(&args, index, name)?;
                if !value.is_finite() {
                    return Err(format!("{} must be finite", name));
                }
                Ok(value)
            }
        }
    };
    let x = coordinate(0, "noise x")?;
    let y = coordinate(1, "noise y")?;
    let z = coordinate(2, "noise z")?;
    Ok(vec![Value::Number(perlin_noise(x, y, z))])
}
fn builtin_random_int(args: Vec<Value>) -> Result<Vec<Value>, String> {
    if args
        .first()
        .is_some_and(|value| matches!(value, Value::Integer(_)))
        || args
            .get(1)
            .is_some_and(|value| matches!(value, Value::Integer(_)))
    {
        return builtin_random_bigint(args);
    }
    let min = integer_argument(args.first().cloned().unwrap_or(Value::Nil), "min")?;
    let max = integer_argument(args.get(1).cloned().unwrap_or(Value::Nil), "max")?;
    if min > max {
        return Err("random min must be less than or equal to max".to_string());
    }
    Ok(vec![Value::Integer(Int::from(
        rand::thread_rng().gen_range(min..=max),
    ))])
}

fn builtin_random_bigint(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let min = bigint_argument(args.first().cloned().unwrap_or(Value::Nil), "min")?;
    let max = bigint_argument(args.get(1).cloned().unwrap_or(Value::Nil), "max")?;
    if min > max {
        return Err("random min must be less than or equal to max".to_string());
    }

    let range = &max - &min;
    let bit_count = range.bits();
    let byte_count = bit_count.div_ceil(8) as usize;
    let excess_bits = (byte_count as u64 * 8).saturating_sub(bit_count);
    let mut bytes = vec![0; byte_count];
    let mut rng = rand::thread_rng();
    let offset = loop {
        rng.fill_bytes(&mut bytes);
        if excess_bits > 0 {
            bytes[0] &= u8::MAX >> excess_bits;
        }
        let offset = BigInt::from_bytes_be(Sign::Plus, &bytes);
        if offset <= range {
            break offset;
        }
    };

    Ok(vec![Value::Integer(Int::from_bigint(min + offset))])
}

fn builtin_table_freeze(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let value = args.first().cloned().unwrap_or(Value::Nil);
    let Value::Table(table) = &value else {
        return Err("table.freeze expects a table".to_string());
    };
    table.borrow_mut().frozen = true;
    Ok(vec![value])
}

fn builtin_table_isfrozen(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Table(table)) = args.first() else {
        return Err("table.isfrozen expects a table".to_string());
    };
    Ok(vec![Value::Bool(table.borrow().frozen)])
}

fn builtin_table_unpack(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Table(table)) = args.first().cloned() else {
        return Err("table.unpack expects a table".to_string());
    };
    Ok(vec![Value::Varargs(table.borrow().array.clone())])
}
fn integer_argument(value: Value, name: &str) -> Result<i64, String> {
    let value = number(value)?;
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i64::MIN as f64
        || value > i64::MAX as f64
    {
        return Err(format!("random {} must be an integer", name));
    }
    Ok(value as i64)
}
fn bigint_argument(value: Value, name: &str) -> Result<BigInt, String> {
    match value {
        Value::Integer(value) => Ok(value.to_bigint()),
        Value::Number(value) if value.is_finite() && value.fract() == 0.0 => {
            BigInt::from_f64(value).ok_or_else(|| format!("random {} must be an integer", name))
        }
        _ => Err(format!("random {} must be an integer", name)),
    }
}
fn builtin_try(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Function(function)) = args.first() else {
        return Err("try expects a function".to_string());
    };
    match &**function {
        Function::Native { call, .. } => match call(args[1..].to_vec()) {
            Ok(mut values) => {
                values.insert(0, Value::Bool(true));
                Ok(values)
            }
            Err(error) => Ok(vec![Value::Bool(false), Value::String(error)]),
        },
        Function::User { .. } => Ok(vec![
            Value::Bool(false),
            Value::String("user function try is unavailable in this base runtime".to_string()),
        ]),
    }
}
fn builtin_require(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    Err("require must be called through the runtime".to_string())
}
