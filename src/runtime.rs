use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{FromPrimitive, ToPrimitive, Zero};
use rand::{Rng, RngCore};

use crate::ast::literal::Literal;
use crate::ast::node_id::NodeId;
use crate::ast::op::{BinOp, UnOp};
use crate::ast::pattern::AssignTarget;
use crate::parser::{Expr, Param, Stmt};

/// The `__`-prefixed primitives the bundled `lib/*.nyk` modules are built
/// on. The register VM bridges this same list, so both engines run the
/// standard library over one set of implementations.
pub(crate) const PRIMITIVES: &[(&str, Native)] = &[
    ("__floor", builtin_floor),
    ("__sqrt", builtin_sqrt),
    ("__ceil", builtin_ceil),
    ("__round", builtin_round),
    ("__sin", builtin_sin),
    ("__cos", builtin_cos),
    ("__tan", builtin_tan),
    ("__asin", builtin_asin),
    ("__acos", builtin_acos),
    ("__atan", builtin_atan),
    ("__atan2", builtin_atan2),
    ("__sinh", builtin_sinh),
    ("__cosh", builtin_cosh),
    ("__tanh", builtin_tanh),
    ("__log", builtin_log),
    ("__log10", builtin_log10),
    ("__pow", builtin_pow),
    ("__fmod", builtin_fmod),
    ("__modf", builtin_modf),
    ("__frexp", builtin_frexp),
    ("__ldexp", builtin_ldexp),
    ("__isfinite", builtin_isfinite),
    ("__isinf", builtin_isinf),
    ("__noise", builtin_noise),
    ("__random_int", builtin_random_int),
    ("__random_bigint", builtin_random_bigint),
    ("__table_unpack", builtin_table_unpack),
    ("__table_freeze", builtin_table_freeze),
    ("__table_isfrozen", builtin_table_isfrozen),
    ("__os_clock", builtin_os_clock),
    ("__os_time", builtin_os_time),
    ("__os_difftime", builtin_os_difftime),
    ("__os_getenv", builtin_os_getenv),
    ("__random_float", builtin_random_float),
];

/// Every `lib/*.nyk` module, keyed by its `@neyuki/...` name. Both engines
/// load their standard library from here.
pub(crate) const BUNDLED_LIBRARIES: &[(&str, &str)] =
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

    // The low 64 bits in two's complement, as an unsigned word.
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
    pub metatable: Option<Rc<RefCell<Table>>>,
}

pub(crate) enum Function {
    Native {
        name: &'static str,
        call: Native,
    },
    User {
        name: Option<String>,
        params: Vec<Param>,
        body: Vec<Stmt>,
        env: EnvRef,
    },
}

#[derive(Clone)]
pub struct RuntimeCallFrame {
    pub name: Option<String>,
    pub source: String,
    pub current_line: usize,
    pub what: &'static str,
    pub num_params: usize,
    pub is_vararg: bool,
    pub(crate) func_val: Option<Value>,
}

thread_local! {
    pub static RUNTIME_CALL_STACK: RefCell<Vec<RuntimeCallFrame>> = const { RefCell::new(Vec::new()) };
}

struct FrameGuard;
impl Drop for FrameGuard {
    fn drop(&mut self) {
        RUNTIME_CALL_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
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
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", path, err))?;
    let program = crate::compiler::compile_source(&source)?;
    let diags = crate::sema::analyze(&program, &source);
    if let Some(err) = diags
        .iter()
        .find(|d| d.severity == crate::diagnostics::severity::Severity::Error)
    {
        return Err(format!("semantic error: {}", err.message));
    }
    let mut runtime = Runtime::new();
    runtime.execute(&program).map(|_| ())
}

pub fn run_source(source: &str) -> Result<(), String> {
    let program = crate::compiler::compile_source(source)?;
    let diags = crate::sema::analyze(&program, source);
    if let Some(err) = diags
        .iter()
        .find(|d| d.severity == crate::diagnostics::severity::Severity::Error)
    {
        return Err(format!("semantic error: {}", err.message));
    }
    let mut runtime = Runtime::new();
    runtime.execute(&program).map(|_| ())
}

pub struct Runtime {
    global: EnvRef,
    call_depth: std::cell::Cell<usize>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
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
            ("tonumber", native("tonumber", builtin_tonumber)),
            ("try", native("try", builtin_try)),
            ("pcall", native("pcall", builtin_pcall)),
            ("xpcall", native("xpcall", builtin_xpcall)),
            ("require", native("require", builtin_require)),
            ("setmetatable", native("setmetatable", builtin_setmetatable)),
            ("getmetatable", native("getmetatable", builtin_getmetatable)),
            ("rawset", native("rawset", builtin_rawset)),
            ("rawget", native("rawget", builtin_rawget)),
            ("rawequal", native("rawequal", builtin_rawequal)),
        ] {
            env.borrow_mut().values.insert(name.to_string(), function);
        }
        for (name, call) in PRIMITIVES
            .iter()
            .chain(crate::string_lib::NATIVES)
            .chain(crate::crypto_lib::NATIVES)
            .chain(crate::fs_lib::NATIVES)
            .chain(crate::http_lib::NATIVES)
            .chain(crate::io_lib::NATIVES)
            .chain(crate::sql_lib::NATIVES)
        {
            env.borrow_mut()
                .values
                .insert(name.to_string(), native(name, *call));
        }
        Self {
            global: env,
            call_depth: std::cell::Cell::new(RUNTIME_CALL_STACK.with(|stack| stack.borrow().len())),
        }
    }

    fn execute(&mut self, program: &[Stmt]) -> Result<Vec<Value>, String> {
        RUNTIME_CALL_STACK.with(|stack| {
            stack.borrow_mut().push(RuntimeCallFrame {
                name: Some("main".to_string()),
                source: "=[runtime]".to_string(),
                current_line: 1,
                what: "main",
                num_params: 0,
                is_vararg: true,
                func_val: None,
            });
        });
        let _guard = FrameGuard;
        let block_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.exec_block(program, self.global.clone())
        }));
        match block_res {
            Ok(Ok(Flow::Return(values))) => Ok(values),
            Ok(Ok(Flow::Normal)) => Ok(Vec::new()),
            Ok(Ok(Flow::Break | Flow::Continue)) => {
                Err("loop control used outside a loop".to_string())
            }
            Ok(Err(err)) => Err(err),
            Err(panic_payload) => {
                let msg = if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unexpected panic during execution".to_string()
                };
                Err(format!("runtime panic caught: {}", msg))
            }
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
                ..
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
                ..
            } => {
                let value = first_value(self.eval(value, env.clone())?);
                self.assign(target, value, env.clone())?;
                if *is_const {
                    self.protect_member(target, env)?;
                }
            }
            Stmt::AssignMany {
                targets, values, ..
            } => {
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
            Stmt::Increment { target, amount, .. } => {
                let target_expr = target.to_expr();
                let current = self.eval(&target_expr, env.clone())?;
                let value = self.numeric(
                    current,
                    BinOp::Add,
                    Value::Integer(Int::from(*amount as i64)),
                )?;
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
                    name: Some(name.clone()),
                    params: params.clone(),
                    body: body.clone(),
                    env: env.clone(),
                }));
                if name.contains('.') {
                    let parts: Vec<&str> = name.split('.').collect();
                    let mut target = Expr::Variable {
                        id: NodeId::next(),
                        name: parts[0].to_string(),
                    };
                    for field in &parts[1..parts.len() - 1] {
                        target = Expr::Member {
                            id: NodeId::next(),
                            object: Box::new(target),
                            field: field.to_string(),
                        };
                    }
                    let final_target = AssignTarget::Member {
                        object: Box::new(target),
                        field: parts.last().unwrap().to_string(),
                    };
                    self.assign(&final_target, value, env.clone())?;
                    if *is_const {
                        self.protect_member(&final_target, env)?;
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
            Stmt::Expr { expr, .. } => {
                self.eval(expr, env)?;
            }
            Stmt::Return { values: exprs, .. } => {
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
                ..
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
            Stmt::While {
                condition, body, ..
            } => {
                while self.eval(condition, env.clone())?.truthy_bool()? {
                    match self.exec_block(body, child(&env))? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow => return Ok(flow),
                    }
                }
            }
            Stmt::Repeat {
                body, condition, ..
            } => loop {
                match self.exec_block(body, child(&env))? {
                    Flow::Normal | Flow::Continue => {}
                    Flow::Break => break,
                    flow => return Ok(flow),
                }
                if self.eval(condition, env.clone())?.truthy_bool()? {
                    break;
                }
            },
            Stmt::For {
                vars, source, body, ..
            } => {
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
                ..
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
            Stmt::Break { .. } => return Ok(Flow::Break),
            Stmt::Continue { .. } => return Ok(Flow::Continue),
            Stmt::Goto { .. } | Stmt::Label { .. } => {}
        }
        Ok(Flow::Normal)
    }

    fn eval(&self, expr: &Expr, env: EnvRef) -> Result<Value, String> {
        match expr {
            Expr::Literal { value: lit, .. } => match lit {
                Literal::Nil => Ok(Value::Nil),
                Literal::Bool(b) => Ok(Value::Bool(*b)),
                Literal::Int(i) => Ok(Value::Integer(Int::from_bigint(i.clone()))),
                Literal::Float(f) => Ok(Value::Number(*f)),
                Literal::String(s) => Ok(Value::String(s.clone())),
            },
            Expr::Interp { parts: value, .. } => self.interpolate(value, env),
            Expr::Variable { name, .. } => {
                lookup(&env, name).ok_or_else(|| format!("undefined name `{}`", name))
            }
            Expr::Vararg { .. } => {
                let values = lookup(&env, "__varargs")
                    .ok_or_else(|| "vararg expression outside a variadic function".to_string())?;
                match values {
                    Value::Table(values) => Ok(Value::Varargs(values.borrow().array.clone())),
                    _ => Ok(values),
                }
            }
            Expr::Member { object, field, .. } => {
                self.index(&self.eval(object, env)?, &Value::String(field.clone()))
            }
            Expr::Index { object, index, .. } => {
                self.index(&self.eval(object, env.clone())?, &self.eval(index, env)?)
            }
            Expr::Table { entries, .. } => {
                let mut table = Table {
                    array: Vec::new(),
                    fields: HashMap::new(),
                    const_fields: HashSet::new(),
                    frozen: false,
                    metatable: None,
                };
                for entry in entries {
                    if matches!(entry.value, Expr::Vararg { .. }) {
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
            Expr::Unary { op, expr, .. } => {
                let value = first_value(self.eval(expr, env)?);
                match op {
                    UnOp::Not => Ok(Value::Bool(!value.truthy_bool()?)),
                    UnOp::Neg => self.number_unary(value, true),
                    UnOp::Len => self.length(value),
                    UnOp::BitNot => match value {
                        Value::Integer(i) => match i {
                            Int::Small(s) => Ok(Value::Integer(Int::Small(!s))),
                            Int::Big(b) => Ok(Value::Integer(Int::from_bigint(!(*b).clone()))),
                        },
                        Value::Number(f) => {
                            let i = f.trunc() as i64;
                            Ok(Value::Integer(Int::Small(!i)))
                        }
                        _ => Err("bitwise not expects an integer".to_string()),
                    },
                }
            }
            Expr::Binary {
                left, op, right, ..
            } => self.binary(left, *op, right, env),
            Expr::Call { callee, args, .. } => {
                let function = self.eval(callee, env.clone())?;
                let values = self.eval_args(args, env)?;
                self.call(function, values).map(collapse_values)
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
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
            Expr::Function { params, body, .. } => Ok(Value::Function(Rc::new(Function::User {
                name: None,
                params: params.clone(),
                body: body.clone(),
                env,
            }))),
        }
    }

    fn eval_args(&self, args: &[Expr], env: EnvRef) -> Result<Vec<Value>, String> {
        let mut values = Vec::new();
        for arg in args {
            if matches!(arg, Expr::Vararg { .. }) {
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
        const MAX_CALL_DEPTH: usize = 128;
        let stack_depth = RUNTIME_CALL_STACK.with(|stack| stack.borrow().len());
        if stack_depth >= MAX_CALL_DEPTH {
            return Err(format!(
                "call stack overflow: exceeded maximum call depth of {}",
                MAX_CALL_DEPTH
            ));
        }
        let depth = self.call_depth.get();
        if depth >= MAX_CALL_DEPTH {
            return Err(format!(
                "call stack overflow: exceeded maximum call depth of {}",
                MAX_CALL_DEPTH
            ));
        }
        self.call_depth.set(depth + 1);
        let res = self.call_inner(function, args);
        self.call_depth.set(depth);
        res
    }

    fn call_inner(&self, function: Value, args: Vec<Value>) -> Result<Vec<Value>, String> {
        match function {
            Value::Function(function) => match &*function {
                Function::Native { name: "try", .. } => self.call_try(args),
                Function::Native {
                    name: "require", ..
                } => self.call_require(args),
                Function::Native { name, call } => {
                    RUNTIME_CALL_STACK.with(|stack| {
                        stack.borrow_mut().push(RuntimeCallFrame {
                            name: Some(name.to_string()),
                            source: "=[C]".to_string(),
                            current_line: 0,
                            what: "C",
                            num_params: 0,
                            is_vararg: true,
                            func_val: Some(Value::Function(function.clone())),
                        });
                    });
                    let _guard = FrameGuard;
                    call(args)
                }
                Function::User {
                    name,
                    params,
                    body,
                    env,
                } => {
                    let call_env = child(env);
                    let mut arg_index = 0;
                    for param in params {
                        if param.variadic {
                            let values = Table {
                                array: args[arg_index..].to_vec(),
                                fields: HashMap::new(),
                                const_fields: HashSet::new(),
                                frozen: false,
                                metatable: None,
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
                    let num_params = params.iter().filter(|p| !p.variadic).count();
                    let is_vararg = params.iter().any(|p| p.variadic);
                    RUNTIME_CALL_STACK.with(|stack| {
                        stack.borrow_mut().push(RuntimeCallFrame {
                            name: name.clone(),
                            source: "=[runtime]".to_string(),
                            current_line: 1,
                            what: "Lua",
                            num_params,
                            is_vararg,
                            func_val: Some(Value::Function(function.clone())),
                        });
                    });
                    let _guard = FrameGuard;
                    match self.exec_block(body, call_env)? {
                        Flow::Return(values) => Ok(values),
                        _ => Ok(vec![Value::Nil]),
                    }
                }
            },
            Value::Table(table) => {
                let mt_call = {
                    let tbl = table.borrow();
                    tbl.metatable
                        .as_ref()
                        .and_then(|mt| mt.borrow().fields.get("__call").cloned())
                };
                if let Some(call_fn) = mt_call {
                    let mut full_args = vec![Value::Table(table)];
                    full_args.extend(args);
                    return self.call(call_fn, full_args);
                }
                Err("value is not callable".to_string())
            }
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

    fn create_json_lib(&self) -> Value {
        let mut tbl = Table {
            array: Vec::new(),
            fields: HashMap::new(),
            const_fields: HashSet::new(),
            frozen: true,
            metatable: None,
        };
        tbl.fields.insert(
            "encode".to_string(),
            native("json.encode", runtime_json_encode),
        );
        tbl.fields.insert(
            "decode".to_string(),
            native("json.decode", runtime_json_decode),
        );
        Value::Table(Rc::new(RefCell::new(tbl)))
    }

    fn create_utf8_lib(&self) -> Value {
        let mut tbl = Table {
            array: Vec::new(),
            fields: HashMap::new(),
            const_fields: HashSet::new(),
            frozen: true,
            metatable: None,
        };
        tbl.fields
            .insert("char".to_string(), native("utf8.char", runtime_utf8_char));
        tbl.fields
            .insert("len".to_string(), native("utf8.len", runtime_utf8_len));
        tbl.fields.insert(
            "codepoint".to_string(),
            native("utf8.codepoint", runtime_utf8_codepoint),
        );
        tbl.fields.insert(
            "offset".to_string(),
            native("utf8.offset", runtime_utf8_offset),
        );
        tbl.fields.insert(
            "charpattern".to_string(),
            Value::String("[\\0-\\x7F\\xC2-\\xFD][\\x80-\\xBF]*".to_string()),
        );
        Value::Table(Rc::new(RefCell::new(tbl)))
    }

    fn create_debug_lib(&self) -> Value {
        let mut tbl = Table {
            array: Vec::new(),
            fields: HashMap::new(),
            const_fields: HashSet::new(),
            frozen: true,
            metatable: None,
        };
        tbl.fields.insert(
            "traceback".to_string(),
            native("debug.traceback", runtime_debug_traceback),
        );
        tbl.fields.insert(
            "getinfo".to_string(),
            native("debug.getinfo", runtime_debug_getinfo),
        );
        Value::Table(Rc::new(RefCell::new(tbl)))
    }

    fn create_coroutine_lib(&self) -> Value {
        let mut tbl = Table {
            array: Vec::new(),
            fields: HashMap::new(),
            const_fields: HashSet::new(),
            frozen: true,
            metatable: None,
        };
        tbl.fields.insert(
            "create".to_string(),
            native("coroutine.create", runtime_coroutine_create),
        );
        tbl.fields.insert(
            "resume".to_string(),
            native("coroutine.resume", runtime_coroutine_resume),
        );
        tbl.fields.insert(
            "yield".to_string(),
            native("coroutine.yield", runtime_coroutine_yield),
        );
        tbl.fields.insert(
            "status".to_string(),
            native("coroutine.status", runtime_coroutine_status),
        );
        tbl.fields.insert(
            "running".to_string(),
            native("coroutine.running", runtime_coroutine_running),
        );
        tbl.fields.insert(
            "wrap".to_string(),
            native("coroutine.wrap", runtime_coroutine_wrap),
        );
        tbl.fields.insert(
            "isyieldable".to_string(),
            native("coroutine.isyieldable", runtime_coroutine_isyieldable),
        );
        Value::Table(Rc::new(RefCell::new(tbl)))
    }

    fn call_require(&self, args: Vec<Value>) -> Result<Vec<Value>, String> {
        let Some(Value::String(package)) = args.first() else {
            return Err("require expects a string path".to_string());
        };
        let clean = package.strip_prefix("@neyuki/").unwrap_or(package.as_str());
        match clean {
            "json" => return Ok(vec![self.create_json_lib()]),
            "utf8" => return Ok(vec![self.create_utf8_lib()]),
            "debug" => return Ok(vec![self.create_debug_lib()]),
            "coroutine" => return Ok(vec![self.create_coroutine_lib()]),
            _ => {}
        }
        let Some((_, source)) = BUNDLED_LIBRARIES.iter().find(|(name, _)| {
            *name == package.as_str() || name.strip_prefix("@neyuki/") == Some(clean)
        }) else {
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

    fn binary(&self, left: &Expr, op: BinOp, right: &Expr, env: EnvRef) -> Result<Value, String> {
        let left = first_value(self.eval(left, env.clone())?);
        if op == BinOp::And || op == BinOp::Or {
            let a = left.truthy_bool()?;
            if (op == BinOp::And && !a) || (op == BinOp::Or && a) {
                return Ok(Value::Bool(a));
            }
            return Ok(Value::Bool(
                first_value(self.eval(right, env)?).truthy_bool()?,
            ));
        }
        let right = first_value(self.eval(right, env)?);

        let metamethod_name = match op {
            BinOp::Add => Some("__add"),
            BinOp::Sub => Some("__sub"),
            BinOp::Mul => Some("__mul"),
            BinOp::Div => Some("__div"),
            BinOp::IDiv => Some("__idiv"),
            BinOp::Mod => Some("__mod"),
            BinOp::Pow => Some("__pow"),
            BinOp::Concat => Some("__concat"),
            BinOp::Eq => Some("__eq"),
            BinOp::Lt => Some("__lt"),
            BinOp::Le => Some("__le"),
            _ => None,
        };

        if let Some(mm) = metamethod_name {
            let find_meta = |v: &Value| -> Option<Value> {
                if let Value::Table(tbl) = v {
                    tbl.borrow()
                        .metatable
                        .as_ref()
                        .and_then(|mt| mt.borrow().fields.get(mm).cloned())
                } else {
                    None
                }
            };
            if let Some(h) = find_meta(&left).or_else(|| find_meta(&right)) {
                let res = self.call(h, vec![left.clone(), right.clone()])?;
                return Ok(collapse_values(res));
            }
        }

        match op {
            BinOp::Coalesce => {
                if matches!(left, Value::Nil) {
                    Ok(right)
                } else {
                    Ok(left)
                }
            }
            BinOp::Concat => Ok(Value::String(format!(
                "{}{}",
                require_string(left)?,
                require_string(right)?
            ))),
            BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::IDiv
            | BinOp::Mod
            | BinOp::Pow => self.numeric(left, op, right),
            BinOp::Eq => Ok(Value::Bool(equal(&left, &right))),
            BinOp::Ne => Ok(Value::Bool(!equal(&left, &right))),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => compare(left, op, right),
            BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::Shl
            | BinOp::Shr
            | BinOp::LShl
            | BinOp::LShr => bitwise(left, op, right),
            BinOp::And | BinOp::Or => unreachable!(),
        }
    }

    fn numeric(&self, left: Value, op: BinOp, right: Value) -> Result<Value, String> {
        if let (Value::Integer(a), Value::Integer(b)) = (&left, &right) {
            if (op == BinOp::IDiv || op == BinOp::Mod) && b.is_zero() {
                return Err("division by zero".to_string());
            }
            return match op {
                BinOp::Add => Ok(Value::Integer(a + b)),
                BinOp::Sub => Ok(Value::Integer(a - b)),
                BinOp::Mul => Ok(Value::Integer(a * b)),
                BinOp::Div => Ok(Value::Number(number(left)? / number(right)?)),
                BinOp::IDiv => Ok(Value::Integer(a.checked_div_floor(b))),
                BinOp::Mod => Ok(Value::Integer(a.mod_floor(b))),
                BinOp::Pow if !b.is_negative() => {
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
        if (op == BinOp::IDiv || op == BinOp::Mod) && b == 0.0 {
            return Err("division by zero".to_string());
        }
        if op == BinOp::Div {
            return Ok(Value::Number(a / b));
        }
        let result = match op {
            BinOp::Add => a + b,
            BinOp::Sub => a - b,
            BinOp::Mul => a * b,
            BinOp::IDiv => (a / b).floor(),
            BinOp::Mod => a - (a / b).floor() * b,
            BinOp::Pow => a.powf(b),
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
        self.index_depth(object, index, 0)
    }

    fn index_depth(&self, object: &Value, index: &Value, depth: usize) -> Result<Value, String> {
        if depth >= 100 {
            return Err(
                "loop in gettable / __index metamethods (depth limit 100 exceeded)".to_string(),
            );
        }
        match object {
            Value::Table(table) => {
                let borrowed = table.borrow();
                let found = match index {
                    Value::String(key) => borrowed.fields.get(key).cloned(),
                    Value::Integer(index) if index.is_positive() => {
                        let idx = index
                            .to_usize()
                            .ok_or_else(|| "table index is too large".to_string())?
                            - 1;
                        borrowed.array.get(idx).cloned()
                    }
                    _ => None,
                };

                if let Some(v) = found
                    && !matches!(v, Value::Nil)
                {
                    return Ok(v);
                }

                let mt_opt = borrowed.metatable.clone();
                drop(borrowed);

                if let Some(mt) = mt_opt {
                    let h_opt = mt.borrow().fields.get("__index").cloned();
                    if let Some(h) = h_opt {
                        match &h {
                            Value::Table(_) => return self.index_depth(&h, index, depth + 1),
                            Value::Function(_) => {
                                let results = self.call(h, vec![object.clone(), index.clone()])?;
                                return Ok(collapse_values(results));
                            }
                            _ => {}
                        }
                    }
                }

                match index {
                    Value::String(_) => Ok(Value::Nil),
                    Value::Integer(i) if i.is_positive() => Ok(Value::Nil),
                    _ => Err("invalid table index".to_string()),
                }
            }
            Value::String(_) => {
                Err("string members require a user-provided string package".to_string())
            }
            _ => Err("value is not indexable".to_string()),
        }
    }
    fn assign(&self, target: &AssignTarget, value: Value, env: EnvRef) -> Result<(), String> {
        match target {
            AssignTarget::Variable(name) => assign_env(&env, name, value),
            AssignTarget::Member { object, field } => self.assign_index(
                &self.eval(object, env)?,
                Value::String(field.clone()),
                value,
            ),
            AssignTarget::Index { object, index } => self.assign_index(
                &self.eval(object, env.clone())?,
                self.eval(index, env)?,
                value,
            ),
        }
    }
    fn assign_index(&self, object: &Value, index: Value, value: Value) -> Result<(), String> {
        self.assign_index_depth(object, index, value, 0)
    }

    fn assign_index_depth(
        &self,
        object: &Value,
        index: Value,
        value: Value,
        depth: usize,
    ) -> Result<(), String> {
        if depth >= 100 {
            return Err(
                "loop in settable / __newindex metamethods (depth limit 100 exceeded)".to_string(),
            );
        }
        match object {
            Value::Table(table) => {
                let has_existing_or_no_mt = {
                    let borrowed = table.borrow();
                    if borrowed.frozen {
                        return Err("assignment to frozen table".to_string());
                    }
                    if borrowed.metatable.is_none() {
                        true
                    } else {
                        match &index {
                            Value::String(k) => borrowed.fields.contains_key(k),
                            Value::Integer(i) if i.is_positive() => {
                                let idx = i.to_usize().unwrap_or(0);
                                idx > 0
                                    && idx <= borrowed.array.len()
                                    && !matches!(borrowed.array[idx - 1], Value::Nil)
                            }
                            _ => false,
                        }
                    }
                };

                if !has_existing_or_no_mt {
                    let mt_newindex = {
                        let borrowed = table.borrow();
                        borrowed
                            .metatable
                            .as_ref()
                            .and_then(|mt| mt.borrow().fields.get("__newindex").cloned())
                    };

                    if let Some(h) = mt_newindex {
                        match &h {
                            Value::Table(_) => {
                                return self.assign_index_depth(&h, index, value, depth + 1);
                            }
                            Value::Function(_) => {
                                self.call(h, vec![object.clone(), index, value])?;
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                }

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

    fn protect_member(&self, target: &AssignTarget, env: EnvRef) -> Result<(), String> {
        let AssignTarget::Member { object, field } = target else {
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
        metatable: None,
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
        Value::Integer(i) => Ok(i.to_string()),
        Value::Number(f) => Ok(f.to_string()),
        _ => Err("expected a string or number".to_string()),
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
fn compare(left: Value, op: BinOp, right: Value) -> Result<Value, String> {
    let result = match (&left, &right) {
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
        _ => number(left)?
            .partial_cmp(&number(right)?)
            .ok_or_else(|| "values are not comparable".to_string())?,
    };
    Ok(Value::Bool(match op {
        BinOp::Lt => result.is_lt(),
        BinOp::Le => result.is_le(),
        BinOp::Gt => result.is_gt(),
        BinOp::Ge => result.is_ge(),
        _ => false,
    }))
}
fn bitwise(left: Value, op: BinOp, right: Value) -> Result<Value, String> {
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
        BinOp::BitAnd => &a & &b,
        BinOp::BitOr => &a | &b,
        BinOp::BitXor => &a ^ &b,
        BinOp::Shl => {
            if let Some(shift) = b.to_i64() {
                if shift < 0 {
                    let u = shift.unsigned_abs();
                    if u > 65536 {
                        if a.is_negative() {
                            Int::from_bigint(BigInt::from(-1))
                        } else {
                            Int::from_bigint(BigInt::from(0))
                        }
                    } else {
                        a.shr(u as usize)
                    }
                } else if shift > 65536 {
                    return Err("shift is too large".to_string());
                } else {
                    a.shl(shift as usize)
                }
            } else {
                return Err("shift is too large".to_string());
            }
        }
        BinOp::Shr => {
            if let Some(shift) = b.to_i64() {
                if shift < 0 {
                    let u = shift.unsigned_abs();
                    if u > 65536 {
                        return Err("shift is too large".to_string());
                    } else {
                        a.shl(u as usize)
                    }
                } else if shift > 65536 {
                    if a.is_negative() {
                        Int::from_bigint(BigInt::from(-1))
                    } else {
                        Int::from_bigint(BigInt::from(0))
                    }
                } else {
                    a.shr(shift as usize)
                }
            } else {
                return Err("shift is too large".to_string());
            }
        }
        BinOp::LShl | BinOp::LShr => {
            // Logical shifts act on the low 64 bits as an unsigned word, so
            // the result is always in 0..2^64 and shifting by 64+ yields 0.
            let word = a.low_u64();
            let bits = b.to_usize().unwrap_or(usize::MAX);
            Int::from_u64(match (op, bits) {
                (_, 64..) => 0,
                (BinOp::LShl, _) => word << bits,
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
    let value = args.into_iter().next().unwrap_or(Value::Nil);
    if let Value::Table(tbl) = &value {
        let tostring_fn = {
            let b = tbl.borrow();
            b.metatable
                .as_ref()
                .and_then(|mt| mt.borrow().fields.get("__tostring").cloned())
        };
        if let Some(func) = tostring_fn {
            let rt = Runtime::new();
            match rt.call(func, vec![value.clone()]) {
                Ok(res) => {
                    if let Some(s) = res.into_iter().next() {
                        return Ok(vec![Value::String(s.to_string())]);
                    }
                }
                Err(err) => return Err(format!("error in __tostring: {}", err)),
            }
        }
    }
    Ok(vec![Value::String(value.to_string())])
}

fn builtin_setmetatable(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let target = args
        .first()
        .ok_or_else(|| "setmetatable expects table as first argument".to_string())?;
    let mt = args
        .get(1)
        .ok_or_else(|| "setmetatable expects metatable as second argument".to_string())?;
    match target {
        Value::Table(tbl) => {
            let mt_table = match mt {
                Value::Nil => None,
                Value::Table(mt_ref) => Some(mt_ref.clone()),
                _ => return Err("metatable must be a table or nil".to_string()),
            };
            tbl.borrow_mut().metatable = mt_table;
            Ok(vec![target.clone()])
        }
        _ => Err("setmetatable expects table as first argument".to_string()),
    }
}

fn builtin_getmetatable(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let target = args
        .first()
        .ok_or_else(|| "getmetatable expects an argument".to_string())?;
    match target {
        Value::Table(tbl) => {
            if let Some(mt) = &tbl.borrow().metatable {
                Ok(vec![Value::Table(mt.clone())])
            } else {
                Ok(vec![Value::Nil])
            }
        }
        _ => Ok(vec![Value::Nil]),
    }
}

fn builtin_rawset(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let target = args
        .first()
        .ok_or_else(|| "rawset expects table as first argument".to_string())?;
    let key = args
        .get(1)
        .ok_or_else(|| "rawset expects key as second argument".to_string())?;
    let val = args.get(2).cloned().unwrap_or(Value::Nil);
    match target {
        Value::Table(tbl) => {
            let mut borrowed = tbl.borrow_mut();
            match key {
                Value::String(s) => {
                    borrowed.fields.insert(s.clone(), val);
                }
                Value::Integer(idx) if idx.is_positive() => {
                    let i = idx
                        .to_usize()
                        .ok_or_else(|| "table index is too large".to_string())?;
                    while borrowed.array.len() < i {
                        borrowed.array.push(Value::Nil);
                    }
                    borrowed.array[i - 1] = val;
                }
                _ => return Err("rawset expects string or integer key".to_string()),
            }
            Ok(vec![target.clone()])
        }
        _ => Err("rawset expects table as first argument".to_string()),
    }
}

fn builtin_rawget(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let target = args
        .first()
        .ok_or_else(|| "rawget expects table as first argument".to_string())?;
    let key = args
        .get(1)
        .ok_or_else(|| "rawget expects key as second argument".to_string())?;
    match target {
        Value::Table(tbl) => {
            let borrowed = tbl.borrow();
            let v = match key {
                Value::String(s) => borrowed.fields.get(s).cloned().unwrap_or(Value::Nil),
                Value::Integer(idx) if idx.is_positive() => {
                    let i = idx
                        .to_usize()
                        .ok_or_else(|| "table index is too large".to_string())?
                        - 1;
                    borrowed.array.get(i).cloned().unwrap_or(Value::Nil)
                }
                _ => Value::Nil,
            };
            Ok(vec![v])
        }
        _ => Err("rawget expects table as first argument".to_string()),
    }
}

fn builtin_rawequal(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let a = args.first().unwrap_or(&Value::Nil);
    let b = args.get(1).unwrap_or(&Value::Nil);
    match (a, b) {
        (Value::Table(t1), Value::Table(t2)) => Ok(vec![Value::Bool(Rc::ptr_eq(t1, t2))]),
        _ => Ok(vec![Value::Bool(equal(a, b))]),
    }
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
fn builtin_tonumber(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let val = args.first().cloned().unwrap_or(Value::Nil);
    let base_opt = args.get(1);

    if let Some(base_val) = base_opt
        && !matches!(base_val, Value::Nil)
    {
        let base = match base_val {
            Value::Integer(i) => i.to_i64().unwrap_or(0),
            Value::Number(f) => *f as i64,
            _ => return Err("bad argument #2 to 'tonumber' (base out of range)".to_string()),
        };
        if !(2..=36).contains(&base) {
            return Err("bad argument #2 to 'tonumber' (base out of range)".to_string());
        }
        let s = match &val {
            Value::String(s) => s.as_str(),
            _ => return Ok(vec![Value::Nil]),
        };
        if s.len() > 65_536 {
            return Ok(vec![Value::Nil]);
        }
        let s_trimmed = s.trim();
        let (sign, s_digits) = if let Some(stripped) = s_trimmed.strip_prefix('-') {
            (-1, stripped.trim_start())
        } else if let Some(stripped) = s_trimmed.strip_prefix('+') {
            (1, stripped.trim_start())
        } else {
            (1, s_trimmed)
        };
        let s_digits = if base == 16 {
            if let Some(stripped) = s_digits
                .strip_prefix("0x")
                .or_else(|| s_digits.strip_prefix("0X"))
            {
                stripped
            } else {
                s_digits
            }
        } else {
            s_digits
        };
        if s_digits.is_empty() {
            return Ok(vec![Value::Nil]);
        }
        return match BigInt::parse_bytes(s_digits.as_bytes(), base as u32) {
            Some(bi) => {
                let bi = if sign < 0 { -bi } else { bi };
                Ok(vec![Value::Integer(Int::from_bigint(bi))])
            }
            None => Ok(vec![Value::Nil]),
        };
    }

    match val {
        Value::Integer(i) => Ok(vec![Value::Integer(i)]),
        Value::Number(f) => Ok(vec![Value::Number(f)]),
        Value::String(s) => {
            if s.len() > 65_536 {
                return Ok(vec![Value::Nil]);
            }
            let s_trimmed = s.trim();
            let (sign, s_rest) = if let Some(stripped) = s_trimmed.strip_prefix('-') {
                (-1, stripped.trim_start())
            } else if let Some(stripped) = s_trimmed.strip_prefix('+') {
                (1, stripped.trim_start())
            } else {
                (1, s_trimmed)
            };
            if let Some(stripped_hex) = s_rest
                .strip_prefix("0x")
                .or_else(|| s_rest.strip_prefix("0X"))
                && !stripped_hex.is_empty()
                && let Some(bi) = BigInt::parse_bytes(stripped_hex.as_bytes(), 16)
            {
                let bi = if sign < 0 { -bi } else { bi };
                return Ok(vec![Value::Integer(Int::from_bigint(bi))]);
            }
            if let Ok(i) = s_trimmed.parse::<i64>() {
                Ok(vec![Value::Integer(Int::from(i))])
            } else if let Ok(bi) = std::str::FromStr::from_str(s_trimmed) {
                Ok(vec![Value::Integer(Int::from_bigint(bi))])
            } else if let Ok(f) = s_trimmed.parse::<f64>() {
                Ok(vec![Value::Number(f)])
            } else {
                Ok(vec![Value::Nil])
            }
        }
        _ => Ok(vec![Value::Nil]),
    }
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
    const MAX_CALL_DEPTH: usize = 128;
    let stack_depth = RUNTIME_CALL_STACK.with(|stack| stack.borrow().len());
    if stack_depth >= MAX_CALL_DEPTH {
        return Ok(vec![
            Value::Bool(false),
            Value::String(format!(
                "call stack overflow: exceeded maximum call depth of {}",
                MAX_CALL_DEPTH
            )),
        ]);
    }
    match &**function {
        Function::Native { call, .. } => match call(args[1..].to_vec()) {
            Ok(mut values) => {
                values.insert(0, Value::Bool(true));
                Ok(values)
            }
            Err(error) => Ok(vec![Value::Bool(false), Value::String(error)]),
        },
        Function::User { .. } => {
            let rt = Runtime::new();
            match rt.call(Value::Function(function.clone()), args[1..].to_vec()) {
                Ok(mut values) => {
                    values.insert(0, Value::Bool(true));
                    Ok(values)
                }
                Err(error) => Ok(vec![Value::Bool(false), Value::String(error)]),
            }
        }
    }
}
fn builtin_pcall(args: Vec<Value>) -> Result<Vec<Value>, String> {
    builtin_try(args)
}
fn builtin_xpcall(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Function(function)) = args.first() else {
        return Err("xpcall expects a function as 1st argument".to_string());
    };
    const MAX_CALL_DEPTH: usize = 128;
    let stack_depth = RUNTIME_CALL_STACK.with(|stack| stack.borrow().len());
    if stack_depth >= MAX_CALL_DEPTH {
        return Ok(vec![
            Value::Bool(false),
            Value::String(format!(
                "call stack overflow: exceeded maximum call depth of {}",
                MAX_CALL_DEPTH
            )),
        ]);
    }
    let err_handler = args.get(1).cloned().unwrap_or(Value::Nil);
    let call_args = if args.len() > 2 {
        args[2..].to_vec()
    } else {
        Vec::new()
    };

    let res = match &**function {
        Function::Native { call, .. } => call(call_args),
        Function::User { .. } => {
            let rt = Runtime::new();
            rt.call(Value::Function(function.clone()), call_args)
        }
    };

    match res {
        Ok(mut values) => {
            values.insert(0, Value::Bool(true));
            Ok(values)
        }
        Err(error) => {
            if let Value::Function(handler_fn) = err_handler {
                let h_res = match &*handler_fn {
                    Function::Native { call, .. } => call(vec![Value::String(error.clone())]),
                    Function::User { .. } => {
                        let rt = Runtime::new();
                        rt.call(
                            Value::Function(handler_fn.clone()),
                            vec![Value::String(error.clone())],
                        )
                    }
                };
                match h_res {
                    Ok(vals) => {
                        let mut out = vec![Value::Bool(false)];
                        out.extend(vals);
                        Ok(out)
                    }
                    Err(h_err) => Ok(vec![Value::Bool(false), Value::String(h_err)]),
                }
            } else {
                Ok(vec![Value::Bool(false), Value::String(error)])
            }
        }
    }
}
fn builtin_require(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    Err("require must be called through the runtime".to_string())
}

fn builtin_os_clock(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(vec![Value::Number(now.as_secs_f64())])
}

fn builtin_os_time(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(vec![Value::Integer(Int::from(now.as_secs() as i64))])
}

fn builtin_os_difftime(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let t2 = number(args.first().cloned().unwrap_or(Value::Nil))?;
    let t1 = number(args.get(1).cloned().unwrap_or(Value::Nil))?;
    Ok(vec![Value::Number(t2 - t1)])
}

fn builtin_os_getenv(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::String(var)) = args.first() else {
        return Err("os.getenv expects a string variable name".to_string());
    };
    if !crate::vm::libs::os::is_env_var_allowed(var) {
        return Ok(vec![Value::Nil]);
    }
    match std::env::var(var) {
        Ok(val) => Ok(vec![Value::String(val)]),
        Err(_) => Ok(vec![Value::Nil]),
    }
}

fn builtin_random_float(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Number(rand::thread_rng().gen_range(0.0..1.0))])
}

fn runtime_to_vm_val(val: &Value, depth: usize) -> Result<crate::vm::value::Value, String> {
    if depth > 256 {
        return Err("JSON nesting depth limit (256) exceeded during encode".to_string());
    }
    match val {
        Value::Nil => Ok(crate::vm::value::Value::Nil),
        Value::Bool(b) => Ok(crate::vm::value::Value::Bool(*b)),
        Value::Number(f) => Ok(crate::vm::value::Value::Float(*f)),
        Value::Integer(i) => Ok(crate::vm::value::Value::from_bigint(i.to_bigint())),
        Value::String(s) => Ok(crate::vm::value::Value::String((s.clone()).into())),
        Value::Table(t) => {
            let borrowed = t.borrow();
            let mut tbl = crate::vm::value::VmTable::new();
            for item in &borrowed.array {
                tbl.array.push(runtime_to_vm_val(item, depth + 1)?);
            }
            for (k, item) in &borrowed.fields {
                tbl.fields.insert(
                    crate::vm::value::StrRef::from(k.as_str()),
                    runtime_to_vm_val(item, depth + 1)?,
                );
            }
            Ok(crate::vm::value::Value::Table(Rc::new(RefCell::new(tbl))))
        }
        _ => Err("cannot serialize function to JSON".to_string()),
    }
}

fn vm_to_runtime_val(val: &crate::vm::value::Value, depth: usize) -> Result<Value, String> {
    if depth > 256 {
        return Err("JSON nesting depth limit (256) exceeded during decode".to_string());
    }
    match val {
        crate::vm::value::Value::Nil => Ok(Value::Nil),
        crate::vm::value::Value::Bool(b) => Ok(Value::Bool(*b)),
        crate::vm::value::Value::Float(f) => Ok(Value::Number(*f)),
        crate::vm::value::Value::Int(i) => Ok(Value::Integer(Int::Small(*i))),
        crate::vm::value::Value::BigInt(i) => Ok(Value::Integer(Int::from_bigint((**i).clone()))),
        crate::vm::value::Value::String(s) => Ok(Value::String(s.to_string())),
        crate::vm::value::Value::Table(t) => {
            let borrowed = t.borrow();
            let mut array = Vec::new();
            for item in &borrowed.array {
                array.push(vm_to_runtime_val(item, depth + 1)?);
            }
            let mut fields = HashMap::new();
            for (k, item) in &borrowed.fields {
                fields.insert(k.to_string(), vm_to_runtime_val(item, depth + 1)?);
            }
            let tbl = Table {
                array,
                fields,
                const_fields: HashSet::new(),
                frozen: borrowed.frozen,
                metatable: None,
            };
            Ok(Value::Table(Rc::new(RefCell::new(tbl))))
        }
        _ => Ok(Value::Nil),
    }
}

fn runtime_json_encode(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let val = args.first().cloned().unwrap_or(Value::Nil);
    let vm_val = runtime_to_vm_val(&val, 0)?;
    let s = crate::vm::libs::json::encode_to_string(&vm_val)?;
    Ok(vec![Value::String(s)])
}

fn runtime_json_decode(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::String(s)) = args.first() else {
        return Err("json.decode expects a string".to_string());
    };
    let vm_val = crate::vm::libs::json::decode_from_str(s)?;
    let runtime_val = vm_to_runtime_val(&vm_val, 0)?;
    Ok(vec![runtime_val])
}

fn runtime_utf8_char(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let mut out = String::new();
    for a in args {
        let cp = match a {
            Value::Integer(i) => match i {
                Int::Small(v) => v as u32,
                Int::Big(b) => b
                    .to_u32()
                    .ok_or_else(|| "codepoint out of range".to_string())?,
            },
            Value::Number(f) => f as u32,
            _ => return Err("utf8.char expects integer codepoints".to_string()),
        };
        let c = char::from_u32(cp).ok_or_else(|| format!("invalid Unicode codepoint {}", cp))?;
        out.push(c);
    }
    Ok(vec![Value::String(out)])
}

fn runtime_utf8_len(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::String(s)) = args.first() else {
        return Err("utf8.len expects string as first argument".to_string());
    };
    let count = s.chars().count();
    Ok(vec![Value::Integer(Int::from(count as i64))])
}

fn runtime_utf8_codepoint(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::String(s)) = args.first() else {
        return Err("utf8.codepoint expects string".to_string());
    };
    let chars: Vec<char> = s.chars().collect();
    let i = args
        .get(1)
        .and_then(|v| match v {
            Value::Integer(Int::Small(idx)) => Some(*idx as usize),
            _ => None,
        })
        .unwrap_or(1);
    let j = args
        .get(2)
        .and_then(|v| match v {
            Value::Integer(Int::Small(idx)) => Some(*idx as usize),
            _ => None,
        })
        .unwrap_or(i);
    let mut results = Vec::new();
    if i >= 1 && i <= chars.len() {
        for idx in i..=j.min(chars.len()) {
            results.push(Value::Integer(Int::from(chars[idx - 1] as i64)));
        }
    }
    Ok(results)
}

fn runtime_utf8_offset(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::String(s)) = args.first() else {
        return Err("utf8.offset expects string".to_string());
    };
    let n = match args.get(1) {
        Some(Value::Integer(Int::Small(i))) => *i,
        _ => return Err("utf8.offset expects n".to_string()),
    };
    if n == 0 {
        return Err("utf8.offset position must not be 0".to_string());
    }
    let char_indices: Vec<(usize, char)> = s.char_indices().collect();
    if n > 0 {
        let idx = (n - 1) as usize;
        if idx < char_indices.len() {
            Ok(vec![Value::Integer(Int::from(
                (char_indices[idx].0 + 1) as i64,
            ))])
        } else if idx == char_indices.len() {
            Ok(vec![Value::Integer(Int::from((s.len() + 1) as i64))])
        } else {
            Ok(vec![Value::Nil])
        }
    } else {
        let count = char_indices.len() as i64;
        let target = count + n;
        if target >= 0 && (target as usize) < char_indices.len() {
            Ok(vec![Value::Integer(Int::from(
                (char_indices[target as usize].0 + 1) as i64,
            ))])
        } else {
            Ok(vec![Value::Nil])
        }
    }
}

fn runtime_debug_traceback(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let mut out = String::new();
    if let Some(msg) = args.first()
        && !matches!(msg, Value::Nil)
    {
        out.push_str(&msg.to_string());
        out.push('\n');
    }
    out.push_str("stack traceback:\n");
    let level_offset = match args.get(1) {
        Some(Value::Integer(i)) => i.to_usize().unwrap_or(1),
        Some(Value::Number(f)) => *f as usize,
        _ => 1,
    };

    RUNTIME_CALL_STACK.with(|stack| {
        let s = stack.borrow();
        let total = s.len();
        for (i, frame) in s.iter().rev().enumerate() {
            if i < level_offset.saturating_sub(1) {
                continue;
            }
            let fn_name = frame.name.as_deref().unwrap_or("<anonymous>");
            let frame_idx = total.saturating_sub(i);
            if frame.what == "main" {
                out.push_str(&format!("  [frame {}] in main chunk\n", frame_idx));
            } else if frame.what == "C" {
                out.push_str(&format!(
                    "  [frame {}] [C]: in function '{}'\n",
                    frame_idx, fn_name
                ));
            } else {
                out.push_str(&format!(
                    "  [frame {}] function '{}' at line {}\n",
                    frame_idx, fn_name, frame.current_line
                ));
            }
        }
    });
    Ok(vec![Value::String(out)])
}

fn runtime_debug_getinfo(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let arg0 = args.first();
    let tbl = match arg0 {
        Some(Value::Integer(i)) => {
            let level = i.to_usize().unwrap_or(0);
            get_runtime_frame_info(level)
        }
        Some(Value::Number(f)) => {
            let level = *f as usize;
            get_runtime_frame_info(level)
        }
        Some(Value::Function(func)) => {
            let mut t = Table {
                array: Vec::new(),
                fields: HashMap::new(),
                const_fields: HashSet::new(),
                frozen: false,
                metatable: None,
            };
            match &**func {
                Function::Native { name, .. } => {
                    t.fields
                        .insert("name".to_string(), Value::String(name.to_string()));
                    t.fields
                        .insert("what".to_string(), Value::String("C".to_string()));
                    t.fields
                        .insert("source".to_string(), Value::String("=[C]".to_string()));
                    t.fields
                        .insert("currentline".to_string(), Value::Integer(Int::from(-1i64)));
                    t.fields
                        .insert("numparams".to_string(), Value::Integer(Int::Small(0)));
                    t.fields.insert("isvararg".to_string(), Value::Bool(true));
                    t.fields
                        .insert("func".to_string(), Value::Function(func.clone()));
                }
                Function::User { name, params, .. } => {
                    let fn_name = name.clone().unwrap_or_else(|| "<anonymous>".to_string());
                    t.fields.insert("name".to_string(), Value::String(fn_name));
                    t.fields
                        .insert("what".to_string(), Value::String("Lua".to_string()));
                    t.fields.insert(
                        "source".to_string(),
                        Value::String("=[runtime]".to_string()),
                    );
                    t.fields
                        .insert("currentline".to_string(), Value::Integer(Int::Small(1)));
                    let numparams = params.iter().filter(|p| !p.variadic).count();
                    let isvararg = params.iter().any(|p| p.variadic);
                    t.fields.insert(
                        "numparams".to_string(),
                        Value::Integer(Int::from(numparams as i64)),
                    );
                    t.fields
                        .insert("isvararg".to_string(), Value::Bool(isvararg));
                    t.fields
                        .insert("func".to_string(), Value::Function(func.clone()));
                }
            }
            Some(t)
        }
        None => get_runtime_frame_info(1),
        _ => None,
    };

    match tbl {
        Some(t) => Ok(vec![Value::Table(Rc::new(RefCell::new(t)))]),
        None => Ok(vec![Value::Nil]),
    }
}

fn get_runtime_frame_info(level: usize) -> Option<Table> {
    RUNTIME_CALL_STACK.with(|stack| {
        let s = stack.borrow();
        if level >= s.len() {
            return None;
        }
        let idx = s.len() - 1 - level;
        let frame = &s[idx];
        let mut t = Table {
            array: Vec::new(),
            fields: HashMap::new(),
            const_fields: HashSet::new(),
            frozen: false,
            metatable: None,
        };
        let name = frame
            .name
            .clone()
            .unwrap_or_else(|| "<anonymous>".to_string());
        t.fields.insert("name".to_string(), Value::String(name));
        t.fields
            .insert("what".to_string(), Value::String(frame.what.to_string()));
        t.fields
            .insert("source".to_string(), Value::String(frame.source.clone()));
        t.fields.insert(
            "currentline".to_string(),
            Value::Integer(Int::from(frame.current_line as i64)),
        );
        t.fields.insert(
            "numparams".to_string(),
            Value::Integer(Int::from(frame.num_params as i64)),
        );
        t.fields
            .insert("isvararg".to_string(), Value::Bool(frame.is_vararg));
        if let Some(func) = &frame.func_val {
            t.fields.insert("func".to_string(), func.clone());
        }
        Some(t)
    })
}

struct RuntimeCoroutine {
    vm: crate::vm::machine::VM,
    co_id: usize,
    status: Rc<RefCell<String>>,
}

thread_local! {
    static COROUTINE_REGISTRY: RefCell<HashMap<usize, RuntimeCoroutine>> = RefCell::new(HashMap::new());
    static NEXT_COROUTINE_ID: std::cell::Cell<usize> = const { std::cell::Cell::new(1) };
}

fn runtime_coroutine_create(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Function(func)) = args.first().cloned() else {
        return Err("coroutine.create expects a function".to_string());
    };

    let id = NEXT_COROUTINE_ID.with(|n| {
        let i = n.get();
        n.set(i + 1);
        i
    });

    let (proto, env_opt) = match &*func {
        Function::User {
            name,
            params,
            body,
            env,
        } => {
            let proto = crate::compiler::try_compile_function_to_proto(name.clone(), params, body)?;
            (proto, Some(env.clone()))
        }
        Function::Native { name, .. } => {
            return Err(format!(
                "coroutine.create cannot wrap native function '{}'",
                name
            ));
        }
    };

    let mut vm = crate::vm::machine::VM::new();
    if let Some(env) = env_opt {
        let mut curr_env = Some(env);
        let mut all_vars = Vec::new();
        while let Some(e) = curr_env {
            for (k, v) in &e.borrow().values {
                all_vars.push((k.clone(), v.clone()));
            }
            curr_env = e.borrow().parent.clone();
        }
        all_vars.reverse();
        for (k, v) in all_vars {
            if let Ok(vm_v) = runtime_to_vm_val(&v, 0) {
                vm.globals
                    .insert(crate::vm::value::StrRef::from(k.as_str()), vm_v);
            }
        }
    }

    let closure = crate::vm::value::Value::Closure(Rc::new(crate::vm::value::VmClosure {
        proto: Rc::new(proto),
        upvalues: Vec::new(),
    }));
    let co_id = vm.next_co_id;
    vm.next_co_id += 1;
    let co_state = crate::vm::machine::CoroutineState {
        stack: Vec::new(),
        frames: Vec::new(),
        status: "suspended".to_string(),
        func: closure,
        yield_callee: 0,
        yield_retc: 0,
        yield_values: Vec::new(),
        open_upvalues: Vec::new(),
    };
    vm.coroutines.insert(co_id, Rc::new(RefCell::new(co_state)));

    let status = Rc::new(RefCell::new("suspended".to_string()));
    let handle = RuntimeCoroutine { vm, co_id, status };

    COROUTINE_REGISTRY.with(|reg| {
        reg.borrow_mut().insert(id, handle);
    });

    let mut tbl = Table {
        array: vec![Value::Function(func)],
        fields: HashMap::new(),
        const_fields: HashSet::new(),
        frozen: false,
        metatable: None,
    };
    tbl.fields
        .insert("__type".to_string(), Value::String("thread".to_string()));
    tbl.fields
        .insert("_id".to_string(), Value::Integer(Int::from(id as i64)));
    tbl.fields
        .insert("status".to_string(), Value::String("suspended".to_string()));

    Ok(vec![Value::Table(Rc::new(RefCell::new(tbl)))])
}

fn runtime_coroutine_resume(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Table(t)) = args.first().cloned() else {
        return Err("coroutine.resume expects a thread".to_string());
    };

    let id = match t.borrow().fields.get("_id") {
        Some(Value::Integer(i)) => i.to_usize().unwrap_or(0),
        _ => 0,
    };

    if id == 0 {
        return Err("coroutine.resume expects a valid thread".to_string());
    }

    let current_status = COROUTINE_REGISTRY
        .with(|reg| reg.borrow().get(&id).map(|h| h.status.borrow().clone()))
        .unwrap_or_else(|| "dead".to_string());

    if current_status == "dead" {
        return Ok(vec![
            Value::Bool(false),
            Value::String("cannot resume dead coroutine".to_string()),
        ]);
    }
    if current_status == "running" {
        return Ok(vec![
            Value::Bool(false),
            Value::String("cannot resume running coroutine".to_string()),
        ]);
    }

    let mut vm_args = vec![crate::vm::value::Value::Table(Rc::new(RefCell::new({
        let mut tbl = crate::vm::value::VmTable::new();
        let co_id =
            COROUTINE_REGISTRY.with(|reg| reg.borrow().get(&id).map(|h| h.co_id).unwrap_or(0));
        tbl.set_str(
            "_id",
            crate::vm::value::Value::from_bigint(num_bigint::BigInt::from(co_id)),
        );
        tbl.set_str("status", crate::vm::value::Value::str("suspended"));
        tbl
    })))];

    for a in &args[1..] {
        vm_args.push(runtime_to_vm_val(a, 0)?);
    }

    let res = COROUTINE_REGISTRY.with(|reg| -> Result<Vec<crate::vm::value::Value>, String> {
        let mut b = reg.borrow_mut();
        let coro = b
            .get_mut(&id)
            .ok_or_else(|| "coroutine not found in registry".to_string())?;
        *coro.status.borrow_mut() = "running".to_string();
        crate::vm::libs::coroutine::coroutine_resume(&mut coro.vm, &vm_args)
    });

    match res {
        Ok(vm_vals) => {
            let is_ok = matches!(vm_vals.first(), Some(crate::vm::value::Value::Bool(true)));
            let new_status = if is_ok {
                COROUTINE_REGISTRY.with(|reg| {
                    let b = reg.borrow();
                    b.get(&id)
                        .and_then(|h| h.vm.coroutines.get(&h.co_id))
                        .map(|c| c.borrow().status.clone())
                        .unwrap_or_else(|| "dead".to_string())
                })
            } else {
                "dead".to_string()
            };

            COROUTINE_REGISTRY.with(|reg| {
                if let Some(h) = reg.borrow().get(&id) {
                    *h.status.borrow_mut() = new_status.clone();
                }
                if new_status == "dead" {
                    reg.borrow_mut().remove(&id);
                }
            });
            t.borrow_mut()
                .fields
                .insert("status".to_string(), Value::String(new_status));

            let mut out = Vec::new();
            for v in vm_vals {
                out.push(vm_to_runtime_val(&v, 0)?);
            }
            Ok(out)
        }
        Err(err) => {
            COROUTINE_REGISTRY.with(|reg| {
                if let Some(h) = reg.borrow().get(&id) {
                    *h.status.borrow_mut() = "dead".to_string();
                }
                reg.borrow_mut().remove(&id);
            });
            t.borrow_mut()
                .fields
                .insert("status".to_string(), Value::String("dead".to_string()));
            Ok(vec![Value::Bool(false), Value::String(err)])
        }
    }
}

fn runtime_coroutine_yield(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    Err("attempt to yield from outside a coroutine".to_string())
}

fn runtime_coroutine_status(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Table(t)) = args.first() else {
        return Err("coroutine.status expects a thread".to_string());
    };
    let id = match t.borrow().fields.get("_id") {
        Some(Value::Integer(i)) => i.to_usize().unwrap_or(0),
        _ => 0,
    };
    if id > 0
        && let Some(st) =
            COROUTINE_REGISTRY.with(|reg| reg.borrow().get(&id).map(|h| h.status.borrow().clone()))
    {
        return Ok(vec![Value::String(st)]);
    }
    let st = t
        .borrow()
        .fields
        .get("status")
        .cloned()
        .unwrap_or(Value::String("dead".to_string()));
    Ok(vec![st])
}

fn runtime_coroutine_running(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Nil, Value::Bool(true)])
}

fn runtime_coroutine_isyieldable(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Bool(false)])
}

fn runtime_coroutine_wrap(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let create_res = runtime_coroutine_create(args)?;
    let co_table = create_res.into_iter().next().unwrap();

    let mut wrapper = Table {
        array: Vec::new(),
        fields: HashMap::new(),
        const_fields: HashSet::new(),
        frozen: false,
        metatable: None,
    };
    wrapper.fields.insert("_co".to_string(), co_table);

    let mut mt = Table {
        array: Vec::new(),
        fields: HashMap::new(),
        const_fields: HashSet::new(),
        frozen: false,
        metatable: None,
    };
    mt.fields.insert(
        "__call".to_string(),
        native("wrapped_coroutine_call", runtime_wrapped_coroutine_call),
    );
    wrapper.metatable = Some(Rc::new(RefCell::new(mt)));

    Ok(vec![Value::Table(Rc::new(RefCell::new(wrapper)))])
}

fn runtime_wrapped_coroutine_call(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let Some(Value::Table(wrap_tbl)) = args.first() else {
        return Err("coroutine wrapper called without self".to_string());
    };
    let co_table = wrap_tbl
        .borrow()
        .fields
        .get("_co")
        .cloned()
        .unwrap_or(Value::Nil);
    if matches!(co_table, Value::Nil) {
        return Err("invalid coroutine wrapper".to_string());
    }
    let mut resume_args = vec![co_table];
    resume_args.extend_from_slice(&args[1..]);
    let res = runtime_coroutine_resume(resume_args)?;
    if let Some(Value::Bool(true)) = res.first() {
        Ok(res[1..].to_vec())
    } else {
        let err_msg = res
            .get(1)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "error in coroutine".to_string());
        Err(err_msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_debug_traceback_and_getinfo() {
        let code = "local debug = require(\"@neyuki/debug\")\n\
local string = require(\"@neyuki/string\")\n\
local captured_tb = \"\"\n\
local captured_info = nil\n\
function inner()\n\
    captured_tb = debug.traceback(\"my_error\")\n\
    captured_info = debug.getinfo(1)\n\
end\n\
function outer()\n\
    inner()\n\
end\n\
outer()\n\
assert(string.find(captured_tb, \"my_error\") ~= nil)\n\
assert(string.find(captured_tb, \"inner\") ~= nil)\n\
assert(string.find(captured_tb, \"outer\") ~= nil)\n\
assert(captured_info.name == \"inner\")\n\
assert(captured_info.what == \"Lua\")\n\
assert(captured_info.numparams == 0)";
        let res = run_source(code);
        assert!(res.is_ok(), "failed with error: {:?}", res.err());
    }

    #[test]
    fn test_runtime_coroutine_yield_and_resume() {
        let code = "local coroutine = require(\"@neyuki/coroutine\")\n\
local co = coroutine.create(function(start)\n\
    local a = coroutine.yield(start + 10)\n\
    local b = coroutine.yield(a * 2)\n\
    return b + 5\n\
end)\n\
assert(coroutine.status(co) == \"suspended\")\n\
local ok1, r1 = coroutine.resume(co, 5)\n\
assert(ok1 == true and r1 == 15)\n\
assert(coroutine.status(co) == \"suspended\")\n\
local ok2, r2 = coroutine.resume(co, 7)\n\
assert(ok2 == true and r2 == 14)\n\
assert(coroutine.status(co) == \"suspended\")\n\
local ok3, r3 = coroutine.resume(co, 20)\n\
assert(ok3 == true and r3 == 25)\n\
assert(coroutine.status(co) == \"dead\")";
        let res = run_source(code);
        assert!(res.is_ok(), "failed with error: {:?}", res.err());
    }

    #[test]
    fn test_runtime_coroutine_wrap() {
        let code = "local coroutine = require(\"@neyuki/coroutine\")\n\
local fn = coroutine.wrap(function(x)\n\
    local y = coroutine.yield(x * 3)\n\
    return y + 10\n\
end)\n\
local r1 = fn(4)\n\
assert(r1 == 12)\n\
local r2 = fn(5)\n\
assert(r2 == 15)";
        let res = run_source(code);
        assert!(res.is_ok(), "failed with error: {:?}", res.err());
    }

    #[test]
    fn test_runtime_try_user_function() {
        let code = "local function work(a, b)\n\
  return a * b\n\
end\n\
local ok, val = try(work, 6, 7)\n\
assert(ok == true)\n\
assert(val == 42)\n\
local ok2, err2 = try(function() error(\"failure\") end)\n\
assert(ok2 == false)\n\
assert(err2 == \"failure\")";
        let res = run_source(code);
        assert!(res.is_ok(), "failed with error: {:?}", res.err());
    }

    #[test]
    fn test_runtime_tonumber_and_pcall() {
        let code = "assert(tonumber(\"1010\", 2) == 10)\n\
assert(tonumber(\"ff\", 16) == 255)\n\
assert(tonumber(\"0xFF\") == 255)\n\
assert(tonumber(\"  99  \") == 99)\n\
local ok, res = pcall(function(x) return x + 1 end, 41)\n\
assert(ok == true)\n\
assert(res == 42)\n\
local ok2, err2 = xpcall(function() error(\"err\") end, function(e) return \"caught: \" .. e end)\n\
assert(ok2 == false)\n\
assert(err2 == \"caught: err\")";
        let res = run_source(code);
        assert!(res.is_ok(), "failed with error: {:?}", res.err());
    }

    #[test]
    fn test_runtime_pcall_recursion_overflow() {
        let code = "local captured_err = nil\n\
local function f()\n\
  local ok, err = pcall(f)\n\
  if not ok and captured_err == nil then\n\
    captured_err = err\n\
  end\n\
end\n\
f()\n\
assert(captured_err ~= nil)";
        let res = run_source(code);
        assert!(res.is_ok(), "failed with error: {:?}", res.err());
    }
}
